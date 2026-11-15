//! `PLAUD.AI` v1 file: 512 B header + ChaCha20 body (spec §6.2).
use crate::{Error, Result};
use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use rsa::{Pkcs1v15Encrypt, PrivateKey};

const HEADER_MIN: usize = 512;

fn err(msg: &str) -> Error {
    Error::Device(msg.into())
}

/// Header is at least 512 B (checked by the caller), so the fixed offsets are in range.
fn header_u32(data: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
}

/// Decrypts a downloaded device file into Ogg Opus bytes. Refuses non-v1 headers.
pub fn decrypt_plaud_file(data: &[u8], priv_key: &PrivateKey) -> Result<Vec<u8>> {
    if data.len() < HEADER_MIN || &data[..8] != b"PLAUD.AI" {
        return Err(err("not a PLAUD.AI file"));
    }
    let version = u16::from_le_bytes([data[8], data[9]]);
    if version != 1 {
        return Err(Error::Device(format!(
            "PLAUD.AI header version {version} unsupported"
        )));
    }
    let header = u16::from_le_bytes([data[10], data[11]]) as usize;
    if header < HEADER_MIN || header > data.len() {
        return Err(err("bad PLAUD.AI header length"));
    }
    let mut body = data[header..].to_vec();

    let wrapped = priv_key
        .decrypt(Pkcs1v15Encrypt, &data[256..512])
        .map_err(|_| err("content key: RSA decrypt failed"))?;
    let ck = wrapped
        .last_chunk::<32>()
        .ok_or_else(|| err("content key too short"))?;
    let new =
        || ChaCha20::new_from_slices(ck, &data[132..144]).map_err(|_| err("bad cipher params"));

    // The counter and segment size are device-controlled: use the fallible cipher API (an
    // exhausted keystream would otherwise panic).
    let exhausted = |_| err("keystream exhausted");
    match header_u32(data, 144) as usize {
        0 => {
            let mut c = new()?;
            c.try_seek(u64::from(header_u32(data, 128)) * 64)
                .map_err(exhausted)?;
            c.try_apply_keystream(&mut body).map_err(exhausted)?;
        }
        // Segmented: each segment restarts at counter 0.
        seg => {
            for s in body.chunks_mut(seg) {
                new()?.try_apply_keystream(s).map_err(exhausted)?;
            }
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::testkey::{key, rng};
    use chacha20::ChaCha20;
    use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
    use rsa::Pkcs1v15Encrypt;

    const CK: [u8; 32] = [9; 32];
    const NONCE: [u8; 12] = [4; 12];

    fn header(version: u16, counter: u32, segment: u32) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..8].copy_from_slice(b"PLAUD.AI");
        h[8..10].copy_from_slice(&version.to_le_bytes());
        h[10..12].copy_from_slice(&512u16.to_le_bytes());
        h[128..132].copy_from_slice(&counter.to_le_bytes());
        h[132..144].copy_from_slice(&NONCE);
        h[144..148].copy_from_slice(&segment.to_le_bytes());
        // The wrapped blob carries junk before the content key: only its last 32 bytes count.
        let wrapped = [[1u8; 8].as_slice(), &CK].concat();
        let ct = key()
            .to_public_key()
            .encrypt(&mut rng(), Pkcs1v15Encrypt, &wrapped)
            .unwrap();
        h[256..512].copy_from_slice(&ct);
        h
    }

    fn crypt(buf: &mut [u8], block: u64) {
        let mut c = ChaCha20::new_from_slices(&CK, &NONCE).unwrap();
        c.seek(block * 64);
        c.apply_keystream(buf);
    }

    #[test]
    fn decrypt_plaud_file_roundtrip() {
        let plain: Vec<u8> = (0..250u32).map(|i| i as u8).collect();

        // Unsegmented: one stream starting at the header counter.
        let mut body = plain.clone();
        crypt(&mut body, 3);
        let file = [header(1, 3, 0), body].concat();
        assert_eq!(decrypt_plaud_file(&file, key()).unwrap(), plain);

        // Segmented: every 100 B segment restarts at counter 0.
        let mut body = plain.clone();
        body.chunks_mut(100).for_each(|s| crypt(s, 0));
        let file = [header(1, 3, 100), body].concat();
        assert_eq!(decrypt_plaud_file(&file, key()).unwrap(), plain);

        // Header only: empty audio, not an error.
        assert!(
            decrypt_plaud_file(&header(1, 0, 0), key())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn decrypt_refuses_header_v2() {
        let file = [header(2, 0, 0), vec![0; 64]].concat();
        assert!(decrypt_plaud_file(&file, key()).is_err());
        // Untrusted input: truncated, wrong magic and a bogus header length fail cleanly.
        let ok = [header(1, 0, 0), vec![0; 64]].concat();
        assert!(decrypt_plaud_file(&ok[..511], key()).is_err());
        assert!(decrypt_plaud_file(&[], key()).is_err());
        let mut bad = ok.clone();
        bad[0] = b'X';
        assert!(decrypt_plaud_file(&bad, key()).is_err());
        let mut bad = ok.clone();
        bad[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
        assert!(decrypt_plaud_file(&bad, key()).is_err());
        // Counter at the end of the keystream must be an error, not a cipher panic.
        let wrap = [header(1, u32::MAX, 0), vec![0; 128]].concat();
        assert!(decrypt_plaud_file(&wrap, key()).is_err());
        // A corrupt wrapped key: RSA implicit rejection (rsa 0.10) yields a wrong (or too short)
        // content key, not a padding error; wrong-key audio is garbage the pull's Ogg check refuses.
        let mut bad = ok.clone();
        bad[300] ^= 0xFF;
        assert_ne!(
            decrypt_plaud_file(&bad, key()).ok(),
            decrypt_plaud_file(&ok, key()).ok()
        );
    }
}
