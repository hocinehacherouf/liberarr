//! In-memory fakes of the device ports, plus a synthetic encrypted `PLAUD.AI` file.
use super::ports::*;
use super::protocol::Advert;
use super::testkey::{key, rng};
use crate::audio::ogg::tests::page;
use crate::{Error, Result};
use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher};
use rsa::Pkcs1v15Encrypt;
use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

pub const SERIAL: &str = "0011223344556677";
pub const SESSION: u32 = 1_700_000_000;

pub fn material() -> UnlockMaterial {
    UnlockMaterial {
        serial: SERIAL.into(),
        signature: vec![1; 256],
        private_pem: key().to_pkcs8_pem(LineEnding::LF).expect("pem").to_string(),
        public_pem: key()
            .to_public_key()
            .to_public_key_pem(LineEnding::LF)
            .expect("pem"),
        bind_token: "uid".into(),
    }
}

/// A valid Ogg stream and the same stream as an encrypted device file (counter 0, unsegmented).
pub fn ogg_and_device_file() -> (Vec<u8>, Vec<u8>) {
    let ogg: Vec<u8> = (0..3).flat_map(|i| page(i, &[i as u8; 200])).collect();
    (ogg.clone(), device_file(&ogg))
}

pub fn device_file(plain: &[u8]) -> Vec<u8> {
    let (ck, nonce) = ([9u8; 32], [4u8; 12]);
    let mut h = vec![0u8; 512];
    h[..8].copy_from_slice(b"PLAUD.AI");
    h[8..10].copy_from_slice(&1u16.to_le_bytes());
    h[10..12].copy_from_slice(&512u16.to_le_bytes());
    h[132..144].copy_from_slice(&nonce);
    let ct = key()
        .to_public_key()
        .encrypt(&mut rng(), Pkcs1v15Encrypt, &ck)
        .expect("encrypt");
    h[256..512].copy_from_slice(&ct);
    let mut body = plain.to_vec();
    ChaCha20::new_from_slices(&ck, &nonce)
        .expect("params")
        .apply_keystream(&mut body);
    [h, body].concat()
}

#[derive(Default)]
pub struct State {
    pub adverts: RefCell<Vec<Advert>>,
    pub files: RefCell<Vec<(u32, Vec<u8>)>>,
    /// `(session, start)` of every download call.
    pub downloads: RefCell<Vec<(u32, u32)>>,
    /// The next download delivers this many bytes, then drops.
    pub drop_after: Cell<Option<usize>>,
    /// That drop reports success (a short download) instead of an error.
    pub drop_ok: Cell<bool>,
    pub ignore_offset: Cell<bool>,
    /// This session reports one more byte on the second `list` (still being recorded).
    pub grow: Cell<Option<u32>>,
    pub opened: Cell<u32>,
    lists: Cell<u32>,
}

/// Scanner and connector in one: both see the same device.
#[derive(Clone, Default)]
pub struct FakeDevice(pub Rc<State>);

impl FakeDevice {
    pub fn with_file(file: Vec<u8>) -> Self {
        let d = Self::default();
        d.0.adverts.borrow_mut().push(advert(SERIAL));
        d.0.files.borrow_mut().push((SESSION, file));
        d
    }
}

pub fn advert(serial: &str) -> Advert {
    Advert {
        id: "id".into(),
        serial: serial.into(),
    }
}

impl DeviceScanner for FakeDevice {
    async fn scan(&self, _secs: u64) -> Result<Vec<Advert>> {
        Ok(self.0.adverts.borrow().clone())
    }
}

impl DeviceConnector for FakeDevice {
    type Session = FakeDevice;
    async fn open(&self, _ad: &Advert, _unlock: &UnlockMaterial) -> Result<FakeDevice> {
        self.0.opened.set(self.0.opened.get() + 1);
        Ok(self.clone())
    }
}

impl DeviceSession for FakeDevice {
    async fn list(&mut self) -> Result<Vec<(u32, u32)>> {
        let n = self.0.lists.get();
        self.0.lists.set(n + 1);
        Ok(self
            .0
            .files
            .borrow()
            .iter()
            .map(|(s, f)| {
                let grown = n > 0 && self.0.grow.get() == Some(*s);
                (*s, f.len() as u32 + u32::from(grown))
            })
            .collect())
    }

    async fn download(
        &mut self,
        session: u32,
        start: u32,
        _end: u32,
        sink: &mut dyn FnMut(u32, &[u8]) -> Result<()>,
    ) -> Result<()> {
        self.0.downloads.borrow_mut().push((session, start));
        let files = self.0.files.borrow();
        let file = &files.iter().find(|(s, _)| *s == session).expect("file").1;
        let from = if self.0.ignore_offset.get() {
            0
        } else {
            start as usize
        };
        let limit = self.0.drop_after.take();
        let mut sent = 0;
        for (i, c) in file[from..].chunks(100).enumerate() {
            if limit.is_some_and(|l| sent >= l) {
                if self.0.drop_ok.take() {
                    return Ok(());
                }
                return Err(Error::Device("link dropped".into()));
            }
            sink((from + i * 100) as u32, c)?;
            sent += c.len();
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct MemKeys(pub RefCell<Vec<UnlockMaterial>>);
impl KeyStore for MemKeys {
    fn load(&self) -> Result<Vec<UnlockMaterial>> {
        Ok(self.0.borrow().clone())
    }
    fn save(&self, all: &[UnlockMaterial]) -> Result<()> {
        *self.0.borrow_mut() = all.to_vec();
        Ok(())
    }
}

#[derive(Default)]
pub struct MemLedger {
    pub known: RefCell<HashSet<u32>>,
    /// `(serial_hash, session, size, recording_id)`
    pub records: RefCell<Vec<(String, u32, u32, i64)>>,
    pub fail_record: Cell<bool>,
}
impl DeviceLedger for MemLedger {
    async fn known(&self, _serial_hash: &str) -> Result<HashSet<u32>> {
        Ok(self.known.borrow().clone())
    }
    async fn record(&self, hash: &str, session: u32, size: u32, id: i64) -> Result<()> {
        if self.fail_record.get() {
            return Err(Error::Storage("ledger unavailable".into()));
        }
        self.records
            .borrow_mut()
            .push((hash.into(), session, size, id));
        Ok(())
    }
}

#[derive(Default)]
pub struct MemPartials {
    pub parts: RefCell<HashMap<String, Vec<u8>>>,
    pub raw: RefCell<HashMap<String, Vec<u8>>>,
    pub staged: RefCell<HashMap<String, Vec<u8>>>,
}
impl PartialStore for MemPartials {
    fn len(&self, key: &str) -> Result<u64> {
        Ok(self.parts.borrow().get(key).map_or(0, |p| p.len() as u64))
    }
    fn append(&self, key: &str, data: &[u8]) -> Result<()> {
        self.parts
            .borrow_mut()
            .entry(key.into())
            .or_default()
            .extend_from_slice(data);
        Ok(())
    }
    fn take(&self, key: &str) -> Result<Vec<u8>> {
        Ok(self.parts.borrow().get(key).cloned().unwrap_or_default())
    }
    fn discard(&self, key: &str) -> Result<()> {
        self.parts.borrow_mut().remove(key);
        Ok(())
    }
    fn keep_raw(&self, key: &str, data: &[u8]) -> Result<PathBuf> {
        self.raw.borrow_mut().insert(key.into(), data.to_vec());
        Ok(PathBuf::from("/raw").join(key))
    }
    fn stage_audio(&self, key: &str, ogg: &[u8]) -> Result<PathBuf> {
        self.staged.borrow_mut().insert(key.into(), ogg.to_vec());
        Ok(PathBuf::from("/audio").join(format!("device-{key}.ogg")))
    }
}
