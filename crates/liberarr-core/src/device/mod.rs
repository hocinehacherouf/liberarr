//! Plaud Note Pro device protocol (protocol 20): byte layouts, session crypto and the on-device
//! file format. Pure functions over bytes; the BLE transport lives in `liberarr-infra`.
//!
//! Every parser takes untrusted device bytes and returns `None`/`Err` on short or malformed input.
pub mod crypto;
pub mod file;
pub mod link;
pub mod ports;
pub mod protocol;
pub mod pull;

#[cfg(test)]
pub(crate) mod fakes;

#[cfg(test)]
pub(crate) mod testkey {
    use rand_chacha::ChaCha8Rng;
    use rand_chacha::rand_core::SeedableRng;
    use rsa::PrivateKey;
    use std::sync::OnceLock;

    /// One deterministic 2048-bit key per test binary: keygen is the slow part.
    pub fn key() -> &'static PrivateKey {
        static KEY: OnceLock<PrivateKey> = OnceLock::new();
        KEY.get_or_init(|| {
            PrivateKey::new(&mut ChaCha8Rng::seed_from_u64(7), 2048).expect("keygen")
        })
    }

    pub fn rng() -> ChaCha8Rng {
        ChaCha8Rng::seed_from_u64(11)
    }
}
