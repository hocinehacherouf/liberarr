//! `liberarr pull [--no-process]`: brings new recordings off the device, then processes them.
use super::{Outcome, process_and_report};
use crate::container::{App, keys, partials};
use liberarr_core::Result;
use liberarr_core::device::ports::{DeviceConnector, DeviceSession, KeyStore, UnlockMaterial};
use liberarr_core::device::protocol::Advert;
use liberarr_core::device::pull::{PullRecordings, PullReport};
use liberarr_core::jobs::JobQueue;
use liberarr_infra::ble::{BtleConnector, BtleScanner, default_adapter};
use rust_i18n::t;
use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Where a download stands, reported at most every 250 ms.
#[derive(Clone)]
pub struct Transfer {
    /// Local start time of the recording (`YYYY-MM-DD HH:MM`).
    pub label: String,
    pub done: u64,
    pub total: u64,
    /// Bytes per second.
    pub speed: f64,
    /// Seconds left.
    pub eta: u64,
}

/// Receives the progress of each download, then `None` when it ends.
pub type OnTransfer = Arc<dyn Fn(Option<&Transfer>) + Send + Sync>;

/// Wraps the connector so every download reports its progress (bytes, speed, ETA).
struct Progress<C>(C, OnTransfer);
struct ProgressSession<S>(S, OnTransfer);

impl<C: DeviceConnector> DeviceConnector for Progress<C> {
    type Session = ProgressSession<C::Session>;
    async fn open(&self, ad: &Advert, unlock: &UnlockMaterial) -> Result<Self::Session> {
        Ok(ProgressSession(
            self.0.open(ad, unlock).await?,
            Arc::clone(&self.1),
        ))
    }
}

pub fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.0
}

impl<S: DeviceSession> DeviceSession for ProgressSession<S> {
    async fn list(&mut self) -> Result<Vec<(u32, u32)>> {
        self.0.list().await
    }

    async fn download(
        &mut self,
        session: u32,
        start: u32,
        end: u32,
        sink: &mut dyn FnMut(u32, &[u8]) -> Result<()>,
    ) -> Result<()> {
        // The session id is the recording's start time.
        let label = jiff::Timestamp::from_second(i64::from(session)).map_or_else(
            |_| session.to_string(),
            |t| {
                t.to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%Y-%m-%d %H:%M")
                    .to_string()
            },
        );
        let (t0, total) = (Instant::now(), u64::from(end));
        let (mut got, mut drawn) = (0u64, None::<Instant>);
        let r = self
            .0
            .download(session, start, end, &mut |offset, data| {
                sink(offset, data)?;
                got += data.len() as u64;
                if drawn.is_none_or(|d| d.elapsed() >= Duration::from_millis(250)) {
                    drawn = Some(Instant::now());
                    let speed = got as f64 / t0.elapsed().as_secs_f64().max(0.001);
                    let left = total.saturating_sub(u64::from(start) + got) as f64;
                    (self.1)(Some(&Transfer {
                        label: label.clone(),
                        done: u64::from(start) + got,
                        total,
                        speed,
                        eta: (left / speed.max(1.0)) as u64,
                    }));
                }
                Ok(())
            })
            .await;
        (self.1)(None);
        r
    }
}

/// Scans for the linked devices and pulls their new recordings (queued for processing). `None`
/// when no device is linked.
pub async fn pull(app: &App, on: OnTransfer) -> Result<Option<PullReport>> {
    let keys = keys(&app.cfg);
    if keys.load()?.is_empty() {
        return Ok(None);
    }
    let adapter = default_adapter().await?;
    let connector = Progress(BtleConnector::new(adapter.clone()), on);
    PullRecordings {
        scanner: &BtleScanner::new(adapter),
        connector: &connector,
        keys: &keys,
        ledger: &app.db,
        partials: &partials(&app.cfg),
        ingest: app.ingest(),
        tz: jiff::tz::TimeZone::system(),
    }
    .run(|d| tokio::time::sleep(d))
    .await
    .map(Some)
}

/// The progress line of the CLI, redrawn in place on stderr.
fn print_transfer(t: Option<&Transfer>) {
    match t {
        Some(t) => {
            eprint!(
                "\r{}",
                t!(
                    "pull.progress",
                    label = t.label,
                    done = format!("{:.1}", mb(t.done)),
                    total = format!("{:.1}", mb(t.total)),
                    speed = format!("{:.0}", t.speed / 1000.0),
                    eta = format!("{}:{:02}", t.eta / 60, t.eta % 60)
                )
            );
            let _ = std::io::stderr().flush();
        }
        None => eprintln!(),
    }
}

pub async fn run(app: &App, no_process: bool) -> Outcome {
    super::note_content_language(&app.cfg);
    if !no_process {
        // Once, before any claim: requeues jobs of a dead process and marks this one as a worker.
        app.db.reset_stale().await?;
    }
    eprintln!("{}", t!("pull.searching"));
    let Some(report) = pull(app, Arc::new(print_transfer)).await? else {
        eprintln!("{}", t!("pull.no_device_linked"));
        return Ok(ExitCode::from(2));
    };

    if report.no_device {
        eprintln!("{}", t!("pull.no_device_in_range"));
        return Ok(ExitCode::from(2));
    }
    println!(
        "{}",
        t!(
            "pull.summary",
            pulled = report.pulled.len(),
            skipped = report.skipped_recording.len(),
            failed = report.failed.len()
        )
    );
    for (session, why) in &report.failed {
        println!(
            "{}",
            t!("pull.failed_session", session = session, why = why)
        );
    }
    for (tail, why) in &report.device_errors {
        println!("{}", t!("pull.device_error", tail = tail, why = why));
    }
    let mut ok = report.failed.is_empty() && report.device_errors.is_empty();
    if !no_process {
        ok &= process_and_report(app, &report.pulled).await?;
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
