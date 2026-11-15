//! Ogg page integrity check (capture pattern, CRC, page sequence continuity) and the Ogg Opus splitter.
use crate::{Error, Result};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum OggError {
    #[error("empty Ogg stream")]
    Empty,
    #[error("page {0}: bad capture pattern, version or truncated")]
    BadPage(usize),
    #[error("page {0}: CRC mismatch")]
    BadCrc(usize),
    #[error("page {0}: sequence number not contiguous")]
    BadSequence(usize),
}

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC_TABLE: [u32; 256] = crc_table();

/// CRC of a page with its own CRC field (bytes 22..26) read as zero.
fn page_crc(page: &[u8]) -> u32 {
    page.iter().enumerate().fold(0u32, |c, (i, &b)| {
        let b = if (22..26).contains(&i) { 0 } else { b };
        (c << 8) ^ CRC_TABLE[((c >> 24) as u8 ^ b) as usize]
    })
}

/// Every page must start with `OggS`, carry a valid CRC and follow the previous page's sequence
/// number of its own logical stream (Plaud interleaves its `PLAUD.AI` stream with the Opus one);
/// the data must end exactly on a page boundary.
pub fn verify_ogg(data: &[u8]) -> Result<(), OggError> {
    if data.is_empty() {
        return Err(OggError::Empty);
    }
    let (mut rest, mut last, mut n) = (data, std::collections::HashMap::<u32, u32>::new(), 0usize);
    while !rest.is_empty() {
        let bad = OggError::BadPage(n);
        let head = rest
            .get(..27)
            .filter(|h| &h[..4] == b"OggS" && h[4] == 0)
            .ok_or(bad)?;
        let segs = head[26] as usize;
        let body: usize = rest
            .get(27..27 + segs)
            .ok_or(OggError::BadPage(n))?
            .iter()
            .map(|&s| s as usize)
            .sum();
        let (page, tail) = rest
            .split_at_checked(27 + segs + body)
            .ok_or(OggError::BadPage(n))?;
        if u32::from_le_bytes([page[22], page[23], page[24], page[25]]) != page_crc(page) {
            return Err(OggError::BadCrc(n));
        }
        let serial = u32::from_le_bytes([page[14], page[15], page[16], page[17]]);
        let seq = u32::from_le_bytes([page[18], page[19], page[20], page[21]]);
        if last
            .insert(serial, seq)
            .is_some_and(|p| seq != p.wrapping_add(1))
        {
            return Err(OggError::BadSequence(n));
        }
        (rest, n) = (tail, n + 1);
    }
    Ok(())
}

const NON_OGG_TOO_BIG: &str = "Non-Ogg file above 25 MB: not supported in v2";

/// Pages of a stream already checked by `verify_ogg`.
fn pages(mut rest: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while !rest.is_empty() {
        let body: usize = rest[27..27 + rest[26] as usize]
            .iter()
            .map(|&s| s as usize)
            .sum();
        let (page, tail) = rest.split_at(27 + rest[26] as usize + body);
        out.push(page);
        rest = tail;
    }
    out
}

fn granule(page: &[u8]) -> i64 {
    i64::from_le_bytes(page[6..14].try_into().unwrap_or_default())
}

/// `OpusHead` + `OpusTags` pages, then `pages` renumbered from 2 with the last one marked end of
/// stream, their granule positions moved back by `shift` so the part plays from 0 like a file of its
/// own: a provider then times its segments from the part start, whatever it reads.
fn build_part(headers: &[&[u8]], pages: &[&[u8]], shift: i64) -> Option<Vec<u8>> {
    let mut out = headers.concat();
    for (i, p) in pages.iter().enumerate() {
        let mut p = p.to_vec();
        let g = granule(&p);
        // -1 means no packet ends on this page: keep it.
        if g >= 0 {
            p[6..14].copy_from_slice(&g.checked_sub(shift)?.to_le_bytes());
        }
        p[18..22].copy_from_slice(&(2 + i as u32).to_le_bytes());
        if i + 1 == pages.len() {
            p[5] |= 0x04;
        }
        let crc = page_crc(&p);
        p[22..26].copy_from_slice(&crc.to_le_bytes());
        out.extend(p);
    }
    Some(out)
}

/// Splits Ogg Opus at page boundaries into parts of at most `max_bytes`, each with its own headers.
/// The offset of a part is where its first sample sits in the full recording.
// shortcut: OpusHead and OpusTags must each fit in one page (true for every encoder seen); upgrade if one doesn't.
pub fn split_ogg(data: &[u8], max_bytes: usize) -> Result<Vec<(Vec<u8>, f64)>> {
    if data.len() <= max_bytes {
        return Ok(vec![(data.to_vec(), 0.0)]);
    }
    if !data.starts_with(b"OggS") {
        return Err(Error::Unsupported(NON_OGG_TOO_BIG.into()));
    }
    let bad = |why: &str| Error::Unsupported(format!("Unsupported Ogg Opus file: {why}"));
    verify_ogg(data).map_err(|e| bad(&e.to_string()))?;
    // Plaud files interleave a second `PLAUD.AI` stream: keep only the Opus one.
    let all = pages(data);
    let opus = all
        .iter()
        .find(|p| p[27 + p[26] as usize..].starts_with(b"OpusHead"))
        .map(|p| p[14..18].to_vec());
    let all: Vec<&[u8]> = all
        .into_iter()
        .filter(|p| opus.as_deref().is_none_or(|s| &p[14..18] == s))
        .collect();
    let [head, tags, rest @ ..] = &all[..] else {
        return Err(bad("missing headers"));
    };
    let body = |p: &[u8]| p[27 + p[26] as usize..].to_vec();
    let (head_body, tags_body) = (body(head), body(tags));
    if !head_body.starts_with(b"OpusHead")
        || head_body.len() < 12
        || !tags_body.starts_with(b"OpusTags")
    {
        return Err(bad("not Opus audio"));
    }
    let pre_skip = i64::from(u16::from_le_bytes([head_body[10], head_body[11]]));
    let seconds = |g: i64| (g - pre_skip).max(0) as f64 / 48_000.0;
    let headers = [*head, *tags];
    let header_len = head.len() + tags.len();

    let (mut parts, mut cur, mut cur_len) = (Vec::new(), Vec::<&[u8]>::new(), header_len);
    let (mut start, mut shift, mut prev) = (0.0, 0, Some(pre_skip));
    for p in rest {
        if header_len + p.len() > max_bytes {
            return Err(bad("a page exceeds the maximum size"));
        }
        if cur_len + p.len() > max_bytes {
            // A part may only start on a fresh packet whose start position is known.
            let (true, Some(g)) = (p[5] & 1 == 0, prev) else {
                return Err(bad("no valid cut point"));
            };
            parts.push((
                build_part(&headers, &cur, shift).ok_or_else(|| bad("granule"))?,
                start,
            ));
            // The part keeps the same OpusHead, so its decoder drops `pre_skip` samples at the start
            // (about 6 ms at each cut): negligible for transcript timestamps.
            (cur, cur_len, start, shift) = (Vec::new(), header_len, seconds(g), g - pre_skip);
        }
        cur.push(p);
        cur_len += p.len();
        prev = Some(granule(p)).filter(|g| *g >= 0);
    }
    parts.push((
        build_part(&headers, &cur, shift).ok_or_else(|| bad("granule"))?,
        start,
    ));
    Ok(parts)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Independent bitwise CRC (poly 0x04C11DB7, MSB first, init 0, no final xor) to build pages.
    fn crc(bytes: &[u8]) -> u32 {
        let mut c = 0u32;
        for &b in bytes {
            c ^= (b as u32) << 24;
            for _ in 0..8 {
                c = if c & 0x8000_0000 != 0 {
                    (c << 1) ^ 0x04C1_1DB7
                } else {
                    c << 1
                };
            }
        }
        c
    }

    pub(crate) fn page(seq: u32, body: &[u8]) -> Vec<u8> {
        page_at(seq, 0, body)
    }

    fn page_at(seq: u32, granule: i64, body: &[u8]) -> Vec<u8> {
        let mut p = b"OggS".to_vec();
        p.extend_from_slice(&[0, if seq == 0 { 2 } else { 0 }]);
        p.extend_from_slice(&granule.to_le_bytes());
        p.extend_from_slice(&1234u32.to_le_bytes()); // stream serial
        p.extend_from_slice(&seq.to_le_bytes());
        p.extend_from_slice(&[0; 4]); // crc placeholder
        p.push(1);
        p.push(body.len() as u8);
        p.extend_from_slice(body);
        let c = crc(&p);
        p[22..26].copy_from_slice(&c.to_le_bytes());
        p
    }

    fn stream() -> Vec<u8> {
        [page(0, b"head"), page(1, b"first body"), page(2, b"last")].concat()
    }

    #[test]
    fn verify_ogg_detects_flipped_byte() {
        let ok = stream();
        assert_eq!(verify_ogg(&ok), Ok(()));

        // Flip one byte anywhere: always rejected.
        for i in 0..ok.len() {
            let mut bad = ok.clone();
            bad[i] ^= 0x01;
            assert!(verify_ogg(&bad).is_err(), "flip at {i} accepted");
        }

        // Cut mid-page, trailing junk, empty input, and a sequence gap.
        assert!(verify_ogg(&ok[..ok.len() - 1]).is_err());
        assert!(verify_ogg(&[ok.clone(), vec![0; 3]].concat()).is_err());
        assert_eq!(verify_ogg(&[]), Err(OggError::Empty));
        let gap = [page(0, b"a"), page(2, b"b")].concat();
        assert_eq!(verify_ogg(&gap), Err(OggError::BadSequence(1)));
        // Truncation off a page boundary is an error, never a panic.
        let bounds = [
            page(0, b"head").len(),
            page(0, b"head").len() + page(1, b"first body").len(),
        ];
        for n in (0..ok.len()).filter(|n| !bounds.contains(n)) {
            assert!(verify_ogg(&ok[..n]).is_err(), "cut at {n} accepted");
        }
    }

    const PRE_SKIP: i64 = 312;

    /// Header pages plus 6 data pages of 1 s each (granule = pre_skip + n s).
    fn opus_stream() -> Vec<u8> {
        let mut head = b"OpusHead\x01\x01".to_vec();
        head.extend_from_slice(&(PRE_SKIP as u16).to_le_bytes());
        head.extend_from_slice(&[0; 7]);
        let mut out = [page(0, &head), page(1, b"OpusTags")].concat();
        for n in 1..=6 {
            out.extend(page_at(
                1 + n,
                PRE_SKIP + n as i64 * 48_000,
                &[n as u8; 100],
            ));
        }
        out
    }

    #[test]
    fn split_ogg_keeps_headers_and_offsets() {
        let data = opus_stream();
        let headers = page(0, &[0; 19]).len() + page(1, b"OpusTags").len();
        let max = headers + 2 * page(2, &[0; 100]).len(); // 2 data pages per part

        let parts = split_ogg(&data, max).unwrap();

        assert_eq!(parts.len(), 3);
        let offsets: Vec<f64> = parts.iter().map(|(_, o)| *o).collect();
        assert_eq!(offsets, [0.0, 2.0, 4.0]);
        for (bytes, _) in &parts {
            assert!(bytes.len() <= max);
            assert_eq!(verify_ogg(bytes), Ok(()));
            assert_eq!(&bytes[28..36], b"OpusHead");
        }
        // Every data page is in exactly one part, in order.
        let bodies: Vec<u8> = parts
            .iter()
            .flat_map(|(b, _)| b[headers..].chunks(128).map(|p| p[28]))
            .collect();
        assert_eq!(bodies, [1, 2, 3, 4, 5, 6]);
        // Each part plays from 0: its pages keep 1 s and 2 s after pre_skip, not the absolute time.
        for (bytes, _) in &parts {
            let granules: Vec<i64> = bytes[headers..].chunks(128).map(granule).collect();
            assert_eq!(granules, [PRE_SKIP + 48_000, PRE_SKIP + 2 * 48_000]);
        }
        // Small enough input is returned whole.
        assert_eq!(split_ogg(&data, data.len()).unwrap(), [(data, 0.0)]);
    }

    #[test]
    fn huge_granule_is_refused_not_overflowed() {
        // Low granules first (shift goes negative at the cut), then one near i64::MAX.
        let mut head = b"OpusHead\x01\x01".to_vec();
        head.extend_from_slice(&(PRE_SKIP as u16).to_le_bytes());
        head.extend_from_slice(&[0; 7]);
        let mut data = [page(0, &head), page(1, b"OpusTags")].concat();
        for (n, g) in [100, 200, i64::MAX, i64::MAX].into_iter().enumerate() {
            data.extend(page_at(2 + n as u32, g, &[1; 100]));
        }
        let headers = page(0, &[0; 19]).len() + page(1, b"OpusTags").len();
        let max = headers + 2 * page(2, &[0; 100]).len();
        assert!(matches!(split_ogg(&data, max), Err(Error::Unsupported(_))));
    }

    /// The same page with another stream serial (CRC recomputed).
    fn with_serial(mut p: Vec<u8>, serial: u32) -> Vec<u8> {
        p[14..18].copy_from_slice(&serial.to_le_bytes());
        p[22..26].fill(0);
        let c = crc(&p);
        p[22..26].copy_from_slice(&c.to_le_bytes());
        p
    }

    #[test]
    fn interleaved_second_stream_is_verified_and_dropped_on_split() {
        // Plaud interleaves a `PLAUD.AI` stream (own serial, own sequence) with the Opus one.
        let data = opus_stream();
        let opus = pages(&data);
        let side: Vec<Vec<u8>> = (0..opus.len() as u32)
            .map(|i| with_serial(page(i, b"PLAUD.AI"), 9))
            .collect();
        let mixed: Vec<u8> = opus
            .iter()
            .zip(&side)
            .flat_map(|(a, b)| [a.to_vec(), b.clone()].concat())
            .collect();
        assert_eq!(verify_ogg(&mixed), Ok(()));

        let headers = page(0, &[0; 19]).len() + page(1, b"OpusTags").len();
        let parts = split_ogg(&mixed, headers + 2 * page(2, &[0; 100]).len()).unwrap();
        assert_eq!(parts.len(), 3);
        for (bytes, _) in &parts {
            assert_eq!(verify_ogg(bytes), Ok(()));
            assert!(!bytes.windows(8).any(|w| w == b"PLAUD.AI"));
        }
    }

    #[test]
    fn non_ogg_too_big_is_refused() {
        let junk = vec![7u8; 100];
        assert_eq!(split_ogg(&junk, 100).unwrap(), [(junk.clone(), 0.0)]);
        let err = split_ogg(&junk, 99).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Non-Ogg file above 25 MB: not supported in v2"
        );
    }
}
