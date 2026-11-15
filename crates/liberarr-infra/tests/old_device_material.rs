//! A home written before the rsa 0.10 update must still open. The fixture was produced by the
//! code on `main` with rsa 0.9.10, from a synthetic key generated for this test:
//! `keys.bin` is the key store (secret `fixture-secret`) holding the same key as PKCS#8 and as
//! PKCS#1 PEM; `unlock.bin` and `recording.plaud` are RSA PKCS#1 v1.5 ciphertexts under it.
use liberarr_core::device::crypto::{open_unlock_secret, private_key_from_pem};
use liberarr_core::device::file::decrypt_plaud_file;
use liberarr_core::device::ports::KeyStore;
use liberarr_infra::keystore::FileKeyStore;
use std::collections::BTreeMap;

#[test]
fn material_from_rsa_0_9_still_opens() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rsa-0.9");
    let stored = FileKeyStore::new(dir.join("keys.bin"), "fixture-secret")
        .load()
        .unwrap();
    let pem_kinds: Vec<_> = stored
        .iter()
        .map(|m| m.private_pem.lines().next())
        .collect();
    assert_eq!(
        pem_kinds,
        [
            Some("-----BEGIN PRIVATE KEY-----"),
            Some("-----BEGIN RSA PRIVATE KEY-----")
        ]
    );
    let unlock = std::fs::read(dir.join("unlock.bin")).unwrap();
    let recording = std::fs::read(dir.join("recording.plaud")).unwrap();
    for m in &stored {
        let key = private_key_from_pem(&m.private_pem).unwrap();
        let s = open_unlock_secret(BTreeMap::from([(0, unlock.clone())]), &key).unwrap();
        assert_eq!((s.key, s.nonce, s.aad), ([3; 32], [5; 12], [8; 12]));
        assert_eq!(
            decrypt_plaud_file(&recording, &key).unwrap(),
            b"synthetic audio, not a recording".repeat(4)
        );
    }
}
