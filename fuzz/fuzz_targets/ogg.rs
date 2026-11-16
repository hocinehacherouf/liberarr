#![no_main]

use liberarr_core::audio::ogg::{opus_duration_seconds, split_ogg, verify_ogg};
use libfuzzer_sys::fuzz_target;

/// Ogg CRC-32 (polynomial 0x04C11DB7, MSB first, initial value 0) with the CRC field read as zero.
fn crc(page: &[u8]) -> u32 {
    page.iter().enumerate().fold(0, |mut c, (i, &b)| {
        c ^= u32::from(if (22..26).contains(&i) { 0 } else { b }) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            };
        }
        c
    })
}

/// Writes a valid CRC into every whole page, so mutations reach the code past the CRC check.
fn fix_crcs(data: &mut [u8]) {
    let mut at = 0;
    while let Some(head) = data.get(at..at + 27) {
        let segs = usize::from(head[26]);
        let Some(lacing) = data.get(at + 27..at + 27 + segs) else {
            return;
        };
        let end = at + 27 + segs + lacing.iter().map(|&s| usize::from(s)).sum::<usize>();
        let Some(page) = data.get_mut(at..end) else {
            return;
        };
        let c = crc(page);
        page[22..26].copy_from_slice(&c.to_le_bytes());
        at = end;
    }
}

// Imported or pulled audio is untrusted: none of these may panic.
fuzz_target!(|data: &[u8]| {
    let _ = verify_ogg(data);
    let mut data = data.to_vec();
    fix_crcs(&mut data);
    let _ = verify_ogg(&data);
    let _ = opus_duration_seconds(&data);
    // The last byte picks a size limit below the input length, so the splitter really splits.
    if let Some(&b) = data.last() {
        let _ = split_ogg(&data, data.len() * usize::from(b) / 256);
    }
});
