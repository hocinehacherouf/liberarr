//! Direct device pull over BLE (spec §6.2): scanner, and a connector that unlocks, handshakes and
//! speaks the framed protocol of `liberarr_core::device`.
//!
//! Only commands 1 (handshake), 26 (list), 28 (download) and the unlock frames `0xFE10`/`0xFE12`
//! are ever written to the device; `Link::send` refuses anything else. Errors never carry the
//! serial, signature, keys or audio.
use btleplug::api::bleuuid::uuid_from_u16;
use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::{Stream, StreamExt};
use liberarr_core::device::crypto::{SessionCipher, open_unlock_secret, private_key_from_pem};
use liberarr_core::device::ports::{DeviceConnector, DeviceScanner, DeviceSession, UnlockMaterial};
use liberarr_core::device::protocol::{self as proto, Advert, DataPacket};
use liberarr_core::{Error, Result};
use rsa::PrivateKey;
use std::collections::{BTreeMap, HashMap};
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::Instant;

/// Device → host notifications, and host → device write-without-response (16-bit UUIDs).
const NOTIFY: u16 = 0x2BB0;
const WRITE: u16 = 0x2BB1;
const MANUFACTURER_ID: u16 = 0x005D;
/// Layouts of the list and download replies below this port version are not known.
const MIN_PORT_VERSION: u16 = 7;

fn dev(msg: impl Into<String>) -> Error {
    Error::Device(msg.into())
}

fn ble(what: &str, e: btleplug::Error) -> Error {
    dev(format!("bluetooth: {what}: {e}"))
}

/// The first Bluetooth adapter. Scanner and connector must share it: the connector finds the
/// peripheral the scanner saw in the adapter's cache.
pub async fn default_adapter() -> Result<Adapter> {
    Manager::new()
        .await
        .map_err(|e| {
            ble(
                "no Bluetooth manager (is Bluetooth on, and BlueZ running on Linux?)",
                e,
            )
        })?
        .adapters()
        .await
        .map_err(|e| ble("cannot list adapters", e))?
        .into_iter()
        .next()
        .ok_or_else(|| dev("no Bluetooth adapter found"))
}

pub struct BtleScanner {
    adapter: Adapter,
}

impl BtleScanner {
    pub fn new(adapter: Adapter) -> Self {
        Self { adapter }
    }

    async fn collect(&self, secs: u64) -> Result<Vec<Advert>> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        let mut seen: HashMap<String, Advert> = HashMap::new();
        loop {
            for p in self
                .adapter
                .peripherals()
                .await
                .map_err(|e| ble("cannot list peripherals", e))?
            {
                let Ok(Some(props)) = p.properties().await else {
                    continue;
                };
                let Some((serial, _)) = props
                    .manufacturer_data
                    .get(&MANUFACTURER_ID)
                    .and_then(|m| proto::parse_advert(m))
                else {
                    continue;
                };
                let id = p.id().to_string();
                seen.insert(id.clone(), Advert { id, serial });
            }
            if Instant::now() >= deadline {
                return Ok(seen.into_values().collect());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

impl DeviceScanner for BtleScanner {
    async fn scan(&self, secs: u64) -> Result<Vec<Advert>> {
        self.adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| ble("cannot start scanning", e))?;
        let found = self.collect(secs).await;
        let _ = self.adapter.stop_scan().await;
        found
    }
}

pub struct BtleConnector {
    adapter: Adapter,
}

impl BtleConnector {
    pub fn new(adapter: Adapter) -> Self {
        Self { adapter }
    }
}

impl DeviceConnector for BtleConnector {
    type Session = BtleSession;

    async fn open(&self, ad: &Advert, unlock: &UnlockMaterial) -> Result<BtleSession> {
        // Fail on a bad key before touching the device.
        let key = private_key_from_pem(&unlock.private_pem)?;
        let peripheral = self
            .adapter
            .peripherals()
            .await
            .map_err(|e| ble("cannot list peripherals", e))?
            .into_iter()
            .find(|p| p.id().to_string() == ad.id)
            .ok_or_else(|| dev("the device is no longer in range: scan again"))?;
        peripheral.connect().await.map_err(|e| {
            ble(
                "cannot connect (wake the device and close the Plaud app)",
                e,
            )
        })?;
        let mut link = match Link::attach(peripheral.clone()).await {
            Ok(link) => link,
            Err(e) => {
                let _ = peripheral.disconnect().await;
                return Err(e);
            }
        };
        let up = async {
            link.unlock(unlock, &key).await?;
            link.handshake(&unlock.bind_token).await
        }
        .await;
        match up {
            Ok(()) => Ok(BtleSession(link)),
            Err(e) => {
                link.close().await;
                Err(e)
            }
        }
    }
}

/// An unlocked, handshaken connection. Dropping it disconnects.
pub struct BtleSession(Link);

impl DeviceSession for BtleSession {
    async fn list(&mut self) -> Result<Vec<(u32, u32)>> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u32::try_from(d.as_secs()).unwrap_or(u32::MAX));
        self.0.send(&proto::list_sessions(now, 0)).await?;
        let mut files: Vec<(u32, u32)> = vec![];
        let mut total = None;
        while total.is_none_or(|t| files.len() < t) {
            let reply = self.0.recv_cmd(&[proto::CMD_LIST], 15).await?;
            let Some(page) = proto::parse_list_page(&reply) else {
                continue;
            };
            let total = *total.get_or_insert(usize::from(page.total));
            // `page` is the index of the first entry: anything else is a duplicate or a gap.
            if usize::from(page.page) != files.len() {
                continue;
            }
            if page.entries.is_empty() && files.len() < total {
                return Err(dev("the device sent an empty page of sessions"));
            }
            files.extend(page.entries);
        }
        Ok(files)
    }

    async fn download(
        &mut self,
        session: u32,
        start: u32,
        end: u32,
        sink: &mut dyn FnMut(u32, &[u8]) -> Result<()>,
    ) -> Result<()> {
        self.0.send(&proto::download(session, start, end)).await?;
        let head = self.0.recv_cmd(&[proto::CMD_DOWNLOAD], 15).await?;
        if let Some(status) = proto::parse_download_status(&head).filter(|s| *s != 0) {
            return Err(dev(format!(
                "the device refused the download (status {status})"
            )));
        }
        // Offsets go to the sink as received: the caller checks continuity and decides whether
        // to restart from 0 when the device ignored `start`.
        loop {
            let pkt = self.0.recv(20).await?;
            match proto::parse_data_packet(&pkt) {
                Some(DataPacket::Tail | DataPacket::End) => return Ok(()),
                Some(DataPacket::Data {
                    session: s,
                    offset,
                    bytes,
                }) if s == session => {
                    if let Err(e) = sink(offset, &bytes) {
                        // The device keeps streaming: swallow the rest of this transfer so it
                        // cannot leak into the next download on this link.
                        self.0.drain_transfer().await;
                        return Err(e);
                    }
                }
                _ => {}
            }
        }
    }
}

struct Link {
    p: Peripheral,
    tx: Characteristic,
    rx: Pin<Box<dyn Stream<Item = ValueNotification> + Send>>,
    /// `None` until the unlock succeeded: unlock frames travel in clear.
    cipher: Option<SessionCipher>,
    closed: bool,
}

impl Drop for Link {
    fn drop(&mut self) {
        if let (false, Ok(rt)) = (self.closed, tokio::runtime::Handle::try_current()) {
            let p = self.p.clone();
            rt.spawn(async move {
                let _ = p.disconnect().await;
            });
        }
    }
}

impl Link {
    async fn attach(p: Peripheral) -> Result<Self> {
        p.discover_services()
            .await
            .map_err(|e| ble("cannot discover services", e))?;
        let chars = p.characteristics();
        let notify = chars
            .iter()
            .find(|c| {
                c.uuid == uuid_from_u16(NOTIFY) && c.properties.contains(CharPropFlags::NOTIFY)
            })
            .ok_or_else(|| dev("the device has no notify characteristic 0x2BB0"))?;
        let tx = chars
            .iter()
            .find(|c| c.uuid == uuid_from_u16(WRITE))
            .ok_or_else(|| dev("the device has no write characteristic 0x2BB1"))?
            .clone();
        p.subscribe(notify)
            .await
            .map_err(|e| ble("cannot subscribe", e))?;
        let rx = p
            .notifications()
            .await
            .map_err(|e| ble("cannot read notifications", e))?;
        Ok(Self {
            p,
            tx,
            rx,
            cipher: None,
            closed: false,
        })
    }

    async fn close(&mut self) {
        self.closed = true;
        let _ = self.p.disconnect().await;
    }

    /// Writes one packet, sealed once the session is up. Only the allow-listed packets go out.
    async fn send(&mut self, pkt: &[u8]) -> Result<()> {
        let allowed = match &self.cipher {
            None => matches!(
                proto::frame_id(pkt),
                Some(proto::FRAME_SN_SIGNATURE | proto::FRAME_KEY_EXCHANGE)
            ),
            Some(_) => matches!(
                proto::reply_cmd(pkt),
                Some(proto::CMD_HANDSHAKE | proto::CMD_LIST | proto::CMD_DOWNLOAD)
            ),
        };
        if !allowed {
            return Err(dev(
                "refusing to send a packet that is not on the allow-list",
            ));
        }
        let wire = match &mut self.cipher {
            Some(c) => c.seal(pkt),
            None => pkt.to_vec(),
        };
        self.p
            .write(&self.tx, &wire, WriteType::WithoutResponse)
            .await
            .map_err(|e| ble("write failed", e))
    }

    /// Next packet (decrypted, replays and forgeries dropped) before `deadline`.
    async fn recv_until(&mut self, deadline: Instant) -> Result<Vec<u8>> {
        loop {
            let n = tokio::time::timeout_at(deadline, self.rx.next())
                .await
                .map_err(|_| dev("timed out waiting for the device"))?
                .ok_or_else(|| dev("the device connection was lost"))?;
            if n.uuid != uuid_from_u16(NOTIFY) {
                continue;
            }
            match &mut self.cipher {
                None => return Ok(n.value),
                Some(c) => {
                    if let Some(pkt) = c.open(&n.value) {
                        return Ok(pkt);
                    }
                }
            }
        }
    }

    async fn recv(&mut self, secs: u64) -> Result<Vec<u8>> {
        self.recv_until(Instant::now() + Duration::from_secs(secs))
            .await
    }

    /// Skips packets until one is a reply to one of `want`.
    async fn recv_cmd(&mut self, want: &[u16], secs: u64) -> Result<Vec<u8>> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let p = self.recv_until(deadline).await?;
            if proto::reply_cmd(&p).is_some_and(|c| want.contains(&c)) {
                return Ok(p);
            }
        }
    }

    /// Discards packets until the transfer ends (End/Tail), the link drops, or the device has
    /// been quiet for 2 s.
    async fn drain_transfer(&mut self) {
        while let Ok(p) = self.recv(2).await {
            if matches!(
                proto::parse_data_packet(&p),
                Some(DataPacket::End | DataPacket::Tail)
            ) {
                break;
            }
        }
    }

    /// Next unlock frame (`0xFE11` key request, `0xFE12` key exchange); `None` when the device
    /// stays silent for `secs`. A lost link and an ECC request (`0xFE30`) are errors.
    async fn recv_unlock(&mut self, secs: u64) -> Result<Option<Vec<u8>>> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let p = match self.recv_until(deadline).await {
                Ok(p) => p,
                // `recv_until` fails before the deadline only when the link is gone.
                Err(_) if Instant::now() >= deadline => return Ok(None),
                Err(e) => return Err(e),
            };
            match proto::frame_id(&p) {
                Some(proto::FRAME_KEY_REQUEST | proto::FRAME_KEY_EXCHANGE) => return Ok(Some(p)),
                Some(proto::FRAME_ECC_SESSION) => {
                    return Err(dev(
                        "the device uses protocol 30 (ECC), which is not supported in v2",
                    ));
                }
                _ => {}
            }
        }
    }

    /// Sends `blob` as unlock chunks. After each chunk it listens briefly for the device's frame
    /// and keeps any secret part it carries. Returns the last frame heard.
    async fn send_frames(
        &mut self,
        frame: u16,
        blob: &[u8],
        parts: &mut Secret,
    ) -> Result<Option<Vec<u8>>> {
        // `chunks` counts in one byte.
        if blob.len() > 25_000 {
            return Err(dev("an unlock blob is too large to send"));
        }
        let chunks = proto::chunks(frame, blob);
        let mut last = None;
        for (i, chunk) in chunks.iter().enumerate() {
            self.send(chunk).await?;
            let wait = if i + 1 < chunks.len() { 3 } else { 10 };
            last = self.recv_unlock(wait).await?;
            if let Some(p) = &last {
                parts.absorb(p);
            }
        }
        Ok(last)
    }

    /// Spec §6.2 unlock: serial signature, public key when asked, then the session secret.
    async fn unlock(&mut self, m: &UnlockMaterial, key: &PrivateKey) -> Result<()> {
        let mut parts = Secret::default();
        let last = self
            .send_frames(proto::FRAME_SN_SIGNATURE, &m.signature, &mut parts)
            .await?;
        let asked = last.as_deref().and_then(proto::frame_id) == Some(proto::FRAME_KEY_REQUEST);
        if asked {
            self.send_frames(
                proto::FRAME_KEY_EXCHANGE,
                m.public_pem.as_bytes(),
                &mut parts,
            )
            .await?;
        }
        while !parts.complete() {
            let p = self.recv_unlock(10).await?.ok_or_else(|| {
                dev(format!(
                    "no session secret from the device ({} parts so far)",
                    parts.parts.len()
                ))
            })?;
            parts.absorb(&p);
        }
        let secret = open_unlock_secret(parts.parts, key)?;
        self.cipher = Some(SessionCipher::new(&secret));
        Ok(())
    }

    /// Handshake with the account user id, repeated once when the device answers GET_SSN.
    async fn handshake(&mut self, token: &str) -> Result<()> {
        self.send(&proto::handshake(token, false)).await?;
        let mut reply = self
            .recv_cmd(&[proto::CMD_HANDSHAKE, proto::CMD_GET_SSN], 10)
            .await?;
        if proto::reply_cmd(&reply) == Some(proto::CMD_GET_SSN) {
            self.send(&proto::handshake(token, true)).await?;
            reply = self.recv_cmd(&[proto::CMD_HANDSHAKE], 10).await?;
        }
        let hello = proto::parse_handshake_reply(&reply)
            .ok_or_else(|| dev("the handshake reply is malformed"))?;
        if hello.status != 0 {
            return Err(dev(format!(
                "the device refused the handshake (status {})",
                hello.status
            )));
        }
        if hello.protocol < MIN_PORT_VERSION {
            return Err(dev(format!(
                "device port version {} is not supported",
                hello.protocol
            )));
        }
        Ok(())
    }
}

/// Key-exchange parts collected so far. No `Debug`: it is key material.
#[derive(Default)]
struct Secret {
    parts: BTreeMap<u8, Vec<u8>>,
    total: Option<u8>,
}

impl Secret {
    fn absorb(&mut self, frame: &[u8]) {
        if let Some((total, index, bytes)) = proto::parse_secret_part(frame) {
            self.parts.insert(index, bytes.to_vec());
            self.total = Some(total);
        }
    }

    fn complete(&self) -> bool {
        self.total
            .is_some_and(|t| self.parts.len() >= usize::from(t))
    }
}
