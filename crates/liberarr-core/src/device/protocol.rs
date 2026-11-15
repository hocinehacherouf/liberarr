//! Byte layouts of the BLE protocol (spec §6.2). Builders produce plaintext packets; the session
//! cipher in `crypto` seals them. Parsers never panic on short or malformed input.

pub const CMD_HANDSHAKE: u16 = 1;
/// Device asks for a second handshake (`P = 1`).
pub const CMD_GET_SSN: u16 = 2;
pub const CMD_LIST: u16 = 26;
pub const CMD_DOWNLOAD: u16 = 28;
pub const CMD_TAIL: u16 = 29;

/// Unlock frames: serial signature (host → device), key request, key exchange (both directions).
pub const FRAME_SN_SIGNATURE: u16 = 0xFE10;
pub const FRAME_KEY_REQUEST: u16 = 0xFE11;
pub const FRAME_KEY_EXCHANGE: u16 = 0xFE12;
/// The device wants an ECC session instead: not supported.
pub const FRAME_ECC_SESSION: u16 = 0xFE30;

/// A device seen while scanning.
#[derive(Clone)]
pub struct Advert {
    pub id: String,
    pub serial: String,
}

/// Serials must not reach logs: only the last 4 characters are shown.
impl std::fmt::Debug for Advert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tail = self
            .serial
            .get(self.serial.len().saturating_sub(4)..)
            .unwrap_or("");
        f.debug_struct("Advert")
            .field("id", &self.id)
            .field("serial", &format_args!("…{tail}"))
            .finish()
    }
}

fn u16le(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Manufacturer data for company id 0x005D (company id stripped) → (serial, protocol).
pub fn parse_advert(manufacturer_0x005d: &[u8]) -> Option<(String, u16)> {
    let serial = manufacturer_0x005d.get(9..17)?;
    let protocol = u16le(manufacturer_0x005d, 18)?;
    Some((
        serial.iter().map(|b| format!("{b:02x}")).collect(),
        protocol,
    ))
}

/// Splits `blob` into `[cmd LE][total][index][≤100 B]` packets.
///
/// Panics above 25 500 B (`total` is one byte): callers send a 256 B signature or a PEM key.
pub fn chunks(cmd: u16, blob: &[u8]) -> Vec<Vec<u8>> {
    let total = u8::try_from(blob.len().div_ceil(100)).expect("blob exceeds 255 chunks");
    blob.chunks(100)
        .enumerate()
        .map(|(i, part)| {
            let mut p = cmd.to_le_bytes().to_vec();
            p.extend_from_slice(&[total, i as u8]);
            p.extend_from_slice(part);
            p
        })
        .collect()
}

/// `[0x01][cmd LE][payload]`
pub fn command(id: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![1];
    v.extend_from_slice(&id.to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// `repeat` is the second handshake after a `CMD_GET_SSN` reply. The token is right-padded with
/// `0` (or cut) to 32 bytes.
pub fn handshake(token: &str, repeat: bool) -> Vec<u8> {
    let mut t = token.as_bytes().to_vec();
    t.resize(32, b'0');
    command(CMD_HANDSHAKE, &[&[2, 0, repeat as u8][..], &t].concat())
}

pub fn list_sessions(now: u32, start: u32) -> Vec<u8> {
    let mut p = now.to_le_bytes().to_vec();
    p.extend_from_slice(&start.to_le_bytes());
    p.push(0);
    command(CMD_LIST, &p)
}

pub fn download(session: u32, start: u32, end: u32) -> Vec<u8> {
    let p = [session, start, end].map(u32::to_le_bytes).concat();
    command(CMD_DOWNLOAD, &p)
}

/// Command id of a `[0x01][cmd LE]…` reply.
pub fn reply_cmd(p: &[u8]) -> Option<u16> {
    (*p.first()? == 1).then(|| u16le(p, 1)).flatten()
}

/// Id of an unlock frame (`0xFE1x`), the first two bytes before the session is up.
pub fn frame_id(p: &[u8]) -> Option<u16> {
    u16le(p, 0)
}

#[derive(Debug, PartialEq, Eq)]
pub struct HandshakeReply {
    pub status: u8,
    pub protocol: u16,
}

pub fn parse_handshake_reply(p: &[u8]) -> Option<HandshakeReply> {
    (reply_cmd(p)? == CMD_HANDSHAKE).then_some(())?;
    Some(HandshakeReply {
        status: *p.get(3)?,
        protocol: u16le(p, 4)?,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct ListPage {
    pub total: u16,
    pub page: u16,
    /// (session id = start epoch, size in bytes)
    pub entries: Vec<(u32, u32)>,
}

pub fn parse_list_page(p: &[u8]) -> Option<ListPage> {
    (reply_cmd(p)? == CMD_LIST).then_some(())?;
    Some(ListPage {
        total: u16le(p, 7)?,
        page: u16le(p, 9)?,
        entries: p
            .get(11..)?
            .as_chunks::<10>()
            .0
            .iter()
            .map(|e| {
                (
                    u32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                    u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
                )
            })
            .collect(),
    })
}

/// Status byte of a download head reply; non-zero means refused.
pub fn parse_download_status(p: &[u8]) -> Option<u8> {
    (reply_cmd(p)? == CMD_DOWNLOAD)
        .then(|| p.get(7).copied())
        .flatten()
}

#[derive(Debug, PartialEq, Eq)]
pub enum DataPacket {
    Data {
        session: u32,
        offset: u32,
        bytes: Vec<u8>,
    },
    /// Offset `0xFFFFFFFF`; same layout as `Data`, so it names its session.
    End { session: u32 },
    /// Command 29.
    Tail,
}

impl DataPacket {
    /// Whether this packet ends the download of `session`. An End of another session is a late
    /// packet from an earlier transfer on the same link and must not end this one.
    pub fn ends(&self, session: u32) -> bool {
        match self {
            DataPacket::End { session: s } => *s == session,
            DataPacket::Tail => true,
            DataPacket::Data { .. } => false,
        }
    }
}

/// `[0x02][session u32][offset u32][len u8][bytes]`. A `len` past the packet end is clamped; the
/// caller's offset-continuity check catches the resulting gap.
pub fn parse_data_packet(p: &[u8]) -> Option<DataPacket> {
    match *p.first()? {
        1 => (u16le(p, 1)? == CMD_TAIL).then_some(DataPacket::Tail),
        2 => {
            let (session, offset, len) = (u32le(p, 1)?, u32le(p, 5)?, *p.get(9)? as usize);
            if offset == u32::MAX {
                return Some(DataPacket::End { session });
            }
            let rest = p.get(10..)?;
            Some(DataPacket::Data {
                session,
                offset,
                bytes: rest[..len.min(rest.len())].to_vec(),
            })
        }
        _ => None,
    }
}

/// An unlock key-exchange part `[0xFE12][total][index][bytes]` → (total, index, bytes).
pub fn parse_secret_part(p: &[u8]) -> Option<(u8, u8, &[u8])> {
    (u16le(p, 0)? == FRAME_KEY_EXCHANGE).then_some(())?;
    Some((*p.get(2)?, *p.get(3)?, p.get(4..)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn parse_advert_reads_serial_and_protocol() {
        let m = hex("0271030456010801088810b50319526726441400040101");
        assert_eq!(parse_advert(&m), Some(("8810b50319526726".into(), 20)));
        // Untrusted: every truncation is rejected or parsed, never a panic.
        for n in 0..m.len() {
            let _ = parse_advert(&m[..n]);
        }
        assert_eq!(parse_advert(&m[..19]), None);
        let a = Advert {
            id: "x".into(),
            serial: "8810b50319526726".into(),
        };
        let dbg = format!("{a:?}");
        assert!(dbg.contains("…6726") && !dbg.contains("8810b503"), "{dbg}");
    }

    #[test]
    fn parse_list_page_and_data_packets() {
        // Builders: layouts from spec §6.2.
        let c = chunks(0xFE10, &[7u8; 250]);
        assert_eq!(c.len(), 3);
        assert_eq!(&c[1][..4], &[0x10, 0xFE, 3, 1]);
        assert_eq!((c[0].len(), c[2].len()), (104, 54));
        assert_eq!(command(26, &[9]), vec![1, 26, 0, 9]);
        let h = handshake("abc", true);
        assert_eq!(&h[..6], &[1, 1, 0, 2, 0, 1]);
        assert_eq!(&h[6..], b"abc00000000000000000000000000000");
        assert_eq!(handshake(&"x".repeat(40), false).len(), 6 + 32);
        let l = list_sessions(0x01020304, 5);
        assert_eq!(&l[3..], &[4, 3, 2, 1, 5, 0, 0, 0, 0]);
        let d = download(1, 2, 3);
        assert_eq!(&d[..3], &[1, 28, 0]);
        assert_eq!(&d[3..], &[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);

        // Handshake reply: status @3, protocol @4.
        let mut r = vec![1, 1, 0, 0, 20, 0, 0, 0, 1, 0, 0, 1];
        let hr = parse_handshake_reply(&r).unwrap();
        assert_eq!((hr.status, hr.protocol), (0, 20));
        assert_eq!(reply_cmd(&r), Some(1));
        r[3] = 5;
        assert_eq!(parse_handshake_reply(&r).unwrap().status, 5);
        assert!(parse_handshake_reply(&r[..5]).is_none());

        // List page: total @7, page @9, entries from 11, 10 B each.
        let mut p = vec![1, 26, 0, 0, 0, 0, 0, 3, 0, 1, 0];
        for (s, z) in [(1000u32, 50u32), (2000, 60)] {
            p.extend_from_slice(&s.to_le_bytes());
            p.extend_from_slice(&z.to_le_bytes());
            p.extend_from_slice(&[0, 0]);
        }
        p.extend_from_slice(&[0xAA; 4]); // trailing partial entry is ignored
        let page = parse_list_page(&p).unwrap();
        assert_eq!((page.total, page.page), (3, 1));
        assert_eq!(page.entries, vec![(1000, 50), (2000, 60)]);
        assert!(parse_list_page(&p[..10]).is_none());

        // Data packets: [02][session][offset][len][bytes], end = offset 0xFFFFFFFF, tail = cmd 29.
        let mut d = vec![2];
        d.extend_from_slice(&1000u32.to_le_bytes());
        d.extend_from_slice(&40u32.to_le_bytes());
        d.push(3);
        d.extend_from_slice(&[1, 2, 3]);
        assert_eq!(
            parse_data_packet(&d),
            Some(DataPacket::Data {
                session: 1000,
                offset: 40,
                bytes: vec![1, 2, 3]
            })
        );
        d[5..9].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_data_packet(&d),
            Some(DataPacket::End { session: 1000 })
        );
        assert_eq!(parse_data_packet(&[1, 29, 0]), Some(DataPacket::Tail));
        assert_eq!(parse_data_packet(&[1, 30, 0]), None);

        // Untrusted: no prefix of any packet may panic.
        for pkt in [&p, &d, &r] {
            for n in 0..pkt.len() {
                let _ = (parse_list_page(&pkt[..n]), parse_data_packet(&pkt[..n]));
                let _ = (
                    parse_handshake_reply(&pkt[..n]),
                    parse_secret_part(&pkt[..n]),
                );
            }
        }
        assert_eq!(
            parse_secret_part(&[0x12, 0xFE, 2, 1, 9, 9]),
            Some((2, 1, &[9u8, 9][..]))
        );
        assert_eq!(parse_secret_part(&[0x11, 0xFE, 2, 1, 9]), None);
    }

    #[test]
    fn a_late_end_of_another_session_does_not_end_the_download() {
        let end = |session: u32| {
            let mut p = vec![2];
            p.extend_from_slice(&session.to_le_bytes());
            p.extend_from_slice(&u32::MAX.to_le_bytes());
            p.push(0);
            p
        };
        let data = |session: u32, offset: u32| {
            let mut p = vec![2];
            p.extend_from_slice(&session.to_le_bytes());
            p.extend_from_slice(&offset.to_le_bytes());
            p.extend_from_slice(&[2, 7, 7]);
            p
        };
        // Fake link: session A's End arrives late, while session B is downloading.
        let (a, b) = (1000, 2000);
        let link = [end(a), data(b, 0), data(b, 2), end(b), data(b, 4)];
        let (mut got, mut ended_at) = (vec![], None);
        for (i, raw) in link.iter().enumerate() {
            let pkt = parse_data_packet(raw).unwrap();
            if pkt.ends(b) {
                ended_at = Some(i);
                break;
            }
            if let DataPacket::Data { session, bytes, .. } = pkt
                && session == b
            {
                got.extend(bytes);
            }
        }
        assert_eq!(ended_at, Some(3));
        assert_eq!(got, vec![7; 4]);
        assert!(DataPacket::Tail.ends(b));
    }
}
