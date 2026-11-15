//! Unlock secret and per-session AEAD (spec §6.2, protocol 20).
use crate::{Error, Result};
use chacha20poly1305::ChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use rsa::{Pkcs1v15Encrypt, PrivateKey};
use std::collections::BTreeMap;

fn err(msg: &str) -> Error {
    Error::Device(msg.into())
}

/// The stored device private key, as Plaud returns it: PKCS#8 or PKCS#1 PEM.
pub fn private_key_from_pem(pem: &str) -> Result<PrivateKey> {
    use rsa::pkcs1::DecodeRsaPrivateKey;
    use rsa::pkcs8::DecodePrivateKey;
    PrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| PrivateKey::from_pkcs1_pem(pem))
        .map_err(|_| err("the stored private key is not a valid PEM"))
}

/// Session key material. No `Debug`: it must never reach a log.
pub struct UnlockSecret {
    pub key: [u8; 32],
    pub nonce: [u8; 12],
    pub aad: [u8; 12],
}

/// RSA PKCS#1 v1.5 decrypt the key-exchange parts (in index order) into
/// `key[32] ‖ nonce[12] ‖ aad[12] ‖ AEAD(b"PLAUD.AI")`, and verify the trailing check.
pub fn open_unlock_secret(
    parts: BTreeMap<u8, Vec<u8>>,
    priv_key: &PrivateKey,
) -> Result<UnlockSecret> {
    let blob: Vec<u8> = parts.into_values().flatten().collect();
    let plain = priv_key
        .decrypt(Pkcs1v15Encrypt, &blob)
        .map_err(|_| err("unlock secret: RSA decrypt failed"))?;
    let short = || err("unlock secret too short");
    let (key, rest) = plain.split_first_chunk::<32>().ok_or_else(short)?;
    let (nonce, rest) = rest.split_first_chunk::<12>().ok_or_else(short)?;
    let (aad, check) = rest.split_first_chunk::<12>().ok_or_else(short)?;
    let opened = cipher(key)
        .decrypt(nonce.into(), Payload { msg: check, aad })
        .map_err(|_| err("unlock secret: check does not authenticate"))?;
    if opened != b"PLAUD.AI" {
        return Err(err("unlock secret: check is not PLAUD.AI"));
    }
    Ok(UnlockSecret {
        key: *key,
        nonce: *nonce,
        aad: *aad,
    })
}

fn cipher(key: &[u8; 32]) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(key.into())
}

/// ChaCha20-Poly1305(key, fixed nonce, aad) over `seq u32 LE ‖ packet`, both directions.
pub struct SessionCipher {
    aead: ChaCha20Poly1305,
    nonce: [u8; 12],
    aad: [u8; 12],
    tx_seq: u32,
    rx_seq: Option<u32>,
}

impl SessionCipher {
    pub fn new(s: &UnlockSecret) -> Self {
        Self {
            aead: cipher(&s.key),
            nonce: s.nonce,
            aad: s.aad,
            tx_seq: 2,
            rx_seq: None,
        }
    }

    /// The first sealed packet carries seq 2.
    pub fn seal(&mut self, pkt: &[u8]) -> Vec<u8> {
        let msg = [self.tx_seq.to_le_bytes().as_slice(), pkt].concat();
        self.tx_seq = self.tx_seq.wrapping_add(1);
        self.aead
            .encrypt(
                (&self.nonce).into(),
                Payload {
                    msg: &msg,
                    aad: &self.aad,
                },
            )
            .expect("AEAD seal fails only on inputs beyond 256 GiB")
    }

    /// `None` for anything that does not authenticate, is too short, or whose seq is not above
    /// the last accepted one (replay / reorder).
    pub fn open(&mut self, ct: &[u8]) -> Option<Vec<u8>> {
        let plain = self
            .aead
            .decrypt(
                (&self.nonce).into(),
                Payload {
                    msg: ct,
                    aad: &self.aad,
                },
            )
            .ok()?;
        let (seq, pkt) = plain.split_first_chunk::<4>()?;
        let seq = u32::from_le_bytes(*seq);
        if self.rx_seq.is_some_and(|last| seq <= last) {
            return None;
        }
        self.rx_seq = Some(seq);
        Some(pkt.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::testkey::{key, rng};
    use chacha20poly1305::ChaCha20Poly1305;
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use rsa::Pkcs1v15Encrypt;

    #[test]
    fn private_key_from_pem_accepts_pkcs1_and_pkcs8() {
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::pkcs8::{EncodePrivateKey, LineEnding};
        let pkcs1 = key().to_pkcs1_pem(LineEnding::LF).unwrap();
        let pkcs8 = key().to_pkcs8_pem(LineEnding::LF).unwrap();
        assert!(pkcs1.starts_with("-----BEGIN RSA PRIVATE KEY"));
        assert_eq!(&private_key_from_pem(&pkcs1).unwrap(), key());
        assert_eq!(&private_key_from_pem(&pkcs8).unwrap(), key());
        assert!(private_key_from_pem("not a key").is_err());
    }

    const K: [u8; 32] = [3; 32];
    const N: [u8; 12] = [5; 12];
    const A: [u8; 12] = [8; 12];

    fn aead() -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new_from_slice(&K).unwrap()
    }

    /// Parts as the device sends them: RSA(key ‖ nonce ‖ aad ‖ AEAD(check)), 100 B per part.
    fn device_parts(check: &[u8]) -> BTreeMap<u8, Vec<u8>> {
        let mut plain = [K.as_slice(), &N, &A].concat();
        plain.extend(
            aead()
                .encrypt(
                    (&N).into(),
                    Payload {
                        msg: check,
                        aad: &A,
                    },
                )
                .unwrap(),
        );
        let ct = key()
            .to_public_key()
            .encrypt(&mut rng(), Pkcs1v15Encrypt, &plain)
            .unwrap();
        ct.chunks(100)
            .enumerate()
            .map(|(i, c)| (i as u8, c.to_vec()))
            .collect()
    }

    /// What the device would put on the wire: AEAD(seq LE ‖ packet).
    fn device_seal(seq: u32, pkt: &[u8]) -> Vec<u8> {
        let m = [seq.to_le_bytes().as_slice(), pkt].concat();
        aead()
            .encrypt((&N).into(), Payload { msg: &m, aad: &A })
            .unwrap()
    }

    #[test]
    fn unlock_roundtrip() {
        let s = open_unlock_secret(device_parts(b"PLAUD.AI"), key()).unwrap();
        assert_eq!((s.key, s.nonce, s.aad), (K, N, A));
    }

    #[test]
    fn unlock_rejects_bad_check() {
        assert!(open_unlock_secret(device_parts(b"PLAUD.XX"), key()).is_err());
        // Garbage / truncated parts must fail cleanly, never panic.
        assert!(open_unlock_secret(BTreeMap::new(), key()).is_err());
        assert!(open_unlock_secret(BTreeMap::from([(0, vec![1; 10])]), key()).is_err());
    }

    #[test]
    fn session_cipher_drops_replay() {
        let secret = UnlockSecret {
            key: K,
            nonce: N,
            aad: A,
        };
        let mut c = SessionCipher::new(&secret);

        // First sealed packet uses seq 2, then 3.
        let w = c.seal(b"hello");
        assert_eq!(
            aead()
                .decrypt((&N).into(), Payload { msg: &w, aad: &A })
                .unwrap(),
            [&[2, 0, 0, 0][..], b"hello"].concat()
        );
        let w = c.seal(b"x");
        assert_eq!(
            &aead()
                .decrypt((&N).into(), Payload { msg: &w, aad: &A })
                .unwrap()[..4],
            &[3, 0, 0, 0]
        );

        assert_eq!(c.open(&device_seal(5, b"a")), Some(b"a".to_vec()));
        assert_eq!(c.open(&device_seal(5, b"a")), None, "replay");
        assert_eq!(c.open(&device_seal(4, b"b")), None, "older");
        assert_eq!(c.open(&device_seal(6, b"c")), Some(b"c".to_vec()));
        assert_eq!(c.open(&[1, 2, 3]), None, "undecryptable");
        assert_eq!(c.open(&[]), None);
        // Authenticated but too short to hold a seq.
        let short = aead()
            .encrypt(
                (&N).into(),
                Payload {
                    msg: &[1, 2],
                    aad: &A,
                },
            )
            .unwrap();
        assert_eq!(c.open(&short), None);
        // A flipped ciphertext byte is rejected and does not advance the window.
        let mut bad = device_seal(9, b"d");
        bad[0] ^= 1;
        assert_eq!(c.open(&bad), None);
        assert_eq!(c.open(&device_seal(7, b"e")), Some(b"e".to_vec()));
    }
}
