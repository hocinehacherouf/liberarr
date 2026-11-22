//! SQLite adapters: one `Db` implements every storage port of `liberarr-core`.
mod jobs;
mod repos;
mod schema;

use liberarr_core::{Error, Result};
use rusqlite::types::Type;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior};
use std::fs::{File, OpenOptions, Permissions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

/// Storage errors never carry row content: SQLite messages name tables and columns, not values.
fn storage(e: impl std::fmt::Display) -> Error {
    Error::Storage(e.to_string())
}

/// Timestamps are stored as RFC3339 UTC text.
fn now() -> String {
    jiff::Timestamp::now().to_string()
}

fn parse_ts(col: usize, s: &str) -> rusqlite::Result<jiff::Timestamp> {
    s.parse()
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, Type::Text, Box::new(e)))
}

/// One SQLite connection behind a mutex; every port call runs on the blocking pool.
/// Cheap to clone: clones share the connection.
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
    /// `<db>.workers`, held shared once `reset_stale` ran: marks this process as a live worker.
    workers: Arc<File>,
    /// `<home>/audio`: recordings store their audio path relative to it, so the home can move.
    audio_dir: PathBuf,
}

impl Db {
    /// Opens (creating if needed) the database: WAL, foreign keys, 5 s busy timeout, migrations.
    pub fn open(path: &Path) -> Result<Db> {
        owner_only(path)?;
        let mut conn = Connection::open(path).map_err(storage)?;
        // Before any other statement, so concurrent openers wait instead of failing.
        conn.busy_timeout(Duration::from_millis(5000))
            .map_err(storage)?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(storage)?;
        conn.pragma_update(None, "foreign_keys", true)
            .map_err(storage)?;
        schema::migrate(&mut conn)?;
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".workers");
        let workers = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path)
            .map_err(storage)?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
            workers: Arc::new(workers),
            // Same layout as the composition root (`<home>/liberarr.db`, `<home>/audio`).
            audio_dir: path.parent().unwrap_or(Path::new("")).join("audio"),
        })
    }

    /// Runs `f` on the blocking pool with the connection.
    async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> Result<T> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            // A panic in an earlier call must not wedge the DB: SQLite state is transactional.
            let mut guard = conn.lock().unwrap_or_else(PoisonError::into_inner);
            f(&mut guard)
        })
        .await
        .map_err(storage)?
        .map_err(storage)
    }

    /// Writes a consistent copy of the database to `dest`, a new 0600 file, while other
    /// connections keep writing (`VACUUM INTO`), then checks it. The copy is one file, no `-wal`.
    pub fn snapshot(&self, dest: &Path) -> Result<()> {
        let target = dest
            .to_str()
            .ok_or_else(|| Error::Storage("backup path is not valid UTF-8".into()))?;
        // Created empty first so SQLite never creates the copy with the umask's mode.
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(dest)
            .map_err(storage)?;
        self.conn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .execute("VACUUM INTO ?1", [target])
            .map_err(storage)?;
        let copy =
            Connection::open_with_flags(dest, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(storage)?;
        let check: String = copy
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(storage)?;
        if check != "ok" {
            return Err(Error::Storage(
                "the database copy failed its integrity check".into(),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn user_version(&self) -> Result<i64> {
        let c = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        c.pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(storage)
    }
}

/// The DB holds every transcript: 0600 whatever the umask or the home's mode. SQLite gives
/// `-wal`/`-shm` the main file's mode; existing ones (crash leftovers, older installs) are tightened.
fn owner_only(path: &Path) -> Result<()> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(storage)?;
    for suffix in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        match std::fs::set_permissions(&p, Permissions::from_mode(0o600)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(storage(e)),
            _ => {}
        }
    }
    Ok(())
}

/// Write transactions take the lock up front (`BEGIN IMMEDIATE`) so they never fail on upgrade.
fn write_tx(conn: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use liberarr_core::device::ports::DeviceLedger;
    use liberarr_core::jobs::{JobQueue, JobStatus};
    use liberarr_core::recordings::{NewRecording, RecordingRepo, RecordingStatus, Source};
    use std::path::PathBuf;

    /// A fresh DB file under the OS temp dir, removed on drop (with its WAL side files).
    struct TempDb(PathBuf);
    impl TempDb {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "liberarr-sqlite-{tag}-{}-{}",
                std::process::id(),
                jiff::Timestamp::now().as_nanosecond()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> PathBuf {
            self.0.join("liberarr.db")
        }
    }
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn new_rec(source_ref: &str) -> NewRecording {
        NewRecording {
            title: Some("t".into()),
            source: Source::File,
            source_ref: source_ref.into(),
            audio_path: "/audio/a.ogg".into(),
            duration_s: Some(1.5),
            created_at: "2026-03-01T10:00:00Z".parse().unwrap(),
        }
    }

    /// AGENTS.md rule 9: copying or renaming `LIBERARR_HOME` must not strand old recordings.
    #[tokio::test]
    async fn recording_survives_home_move() {
        use crate::files::{FsAudioStore, FsNoteStore};
        use liberarr_core::exports::ExportToObsidian;
        use liberarr_core::recordings::AudioStore;
        use liberarr_core::summaries::{MeetingSummary, SummaryRepo};
        use liberarr_core::transcription::{Segment, Transcript, TranscriptRepo};

        let t = TempDb::new("move");
        let (a, b) = (t.0.join("a"), t.0.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        let src = t.0.join("in.ogg");
        std::fs::write(&src, b"ogg").unwrap();

        let db = Db::open(&a.join("liberarr.db")).unwrap();
        let stored = FsAudioStore {
            audio: a.join("audio"),
        }
        .store(&src, "fp")
        .unwrap();
        let mut new = new_rec("r");
        new.audio_path = stored.path;
        let id = db.add(new).await.unwrap();
        TranscriptRepo::save(
            &db,
            id,
            &Transcript {
                engine: "e".into(),
                language: None,
                text: "salut".into(),
                segments: vec![Segment {
                    start: 0.0,
                    end: 1.0,
                    text: "salut".into(),
                }],
                has_timestamps: true,
            },
        )
        .await
        .unwrap();
        SummaryRepo::save(
            &db,
            id,
            &MeetingSummary {
                title: "Point".into(),
                summary_md: "x".into(),
                mindmap: "mindmap\n  root".into(),
                actions: vec![],
            },
        )
        .await
        .unwrap();
        let export = |home: &Path, db: &Db| {
            let db = db.clone();
            let vault = home.join("obsidian");
            async move {
                ExportToObsidian {
                    recordings: &db,
                    transcripts: &db,
                    summaries: &db,
                    notes: &FsNoteStore,
                    vault,
                    tz: jiff::tz::TimeZone::UTC,
                    lang: liberarr_core::lang::Language::Fr,
                }
                .run(id)
                .await
                .unwrap()
                .unwrap()
            }
        };
        let first = export(&a, &db).await;
        assert!(first.starts_with(&a));
        drop(db);

        // Move the whole home, as a Mac -> NAS copy would.
        std::fs::rename(&a, &b).unwrap();
        let db = Db::open(&b.join("liberarr.db")).unwrap();
        let rec = RecordingRepo::get(&db, id).await.unwrap().unwrap();
        assert!(rec.audio_path.starts_with(&b), "{:?}", rec.audio_path);
        assert_eq!(std::fs::read(&rec.audio_path).unwrap(), b"ogg");
        let second = export(&b, &db).await;
        assert!(second.starts_with(&b), "{second:?}");
        assert!(!a.exists(), "the old home must not be recreated");
    }

    #[tokio::test]
    async fn migrate_is_idempotent() {
        let t = TempDb::new("migrate");
        let db = Db::open(&t.path()).unwrap();
        let id = db.add(new_rec("h1")).await.unwrap();
        drop(db);
        // Re-opening must neither fail nor touch existing data.
        let db = Db::open(&t.path()).unwrap();
        assert_eq!(db.find_by_ref("h1").await.unwrap(), Some(id));
        assert_eq!(db.user_version().unwrap(), schema::MIGRATIONS.len() as i64);
    }

    #[tokio::test]
    async fn claim_is_exclusive_across_connections() {
        let t = TempDb::new("claim");
        let (a, b) = (Db::open(&t.path()).unwrap(), Db::open(&t.path()).unwrap());
        let rec = a.add(new_rec("h1")).await.unwrap();
        a.enqueue(rec, "transcribe").await.unwrap();

        let (r1, r2, r3, r4) = tokio::join!(
            a.claim_next(),
            b.claim_next(),
            a.claim_next(),
            b.claim_next()
        );
        let claims = [r1, r2, r3, r4]
            .into_iter()
            .filter_map(|r| r.unwrap())
            .count();
        assert_eq!(claims, 1);
    }

    #[tokio::test]
    async fn reset_stale_requeues_running() {
        let t = TempDb::new("stale");
        let db = Db::open(&t.path()).unwrap();
        let rec = db.add(new_rec("h1")).await.unwrap();
        db.enqueue(rec, "transcribe").await.unwrap();
        let job = db.claim_next().await.unwrap().unwrap();
        assert_eq!(job.attempts, 1);
        assert_eq!(
            db.list_items().await.unwrap()[0].status,
            RecordingStatus::Processing
        );

        // No other worker holds the lock: the job is a leftover of a dead process.
        assert_eq!(db.reset_stale().await.unwrap(), 1);
        assert_eq!(db.list_for(rec).await.unwrap()[0].status, JobStatus::Queued);

        // Claimed again, then failed: the read model reports the failing step.
        let job = db.claim_next().await.unwrap().unwrap();
        assert_eq!(job.attempts, 2);
        db.fail(&job, "boom").await.unwrap();
        let item = &db.list_items().await.unwrap()[0];
        assert_eq!(item.status, RecordingStatus::Failed);
        assert_eq!(item.failed_step.as_deref(), Some("transcribe"));
    }

    #[tokio::test]
    async fn reset_stale_fails_a_job_that_keeps_crashing() {
        let t = TempDb::new("crashloop");
        let rec = {
            let db = Db::open(&t.path()).unwrap();
            let rec = db.add(new_rec("h1")).await.unwrap();
            db.enqueue(rec, "transcribe").await.unwrap();
            rec
        };
        for attempt in 1..=3 {
            let db = Db::open(&t.path()).unwrap();
            assert_eq!(db.reset_stale().await.unwrap(), u64::from(attempt > 1));
            assert_eq!(db.claim_next().await.unwrap().unwrap().attempts, attempt);
        }
        let db = Db::open(&t.path()).unwrap();
        assert_eq!(db.reset_stale().await.unwrap(), 0);
        let job = &db.list_for(rec).await.unwrap()[0];
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.error.as_deref(), Some("stopped after repeated crashes"));
    }

    #[tokio::test]
    async fn reset_stale_spares_jobs_of_live_worker() {
        let t = TempDb::new("live");
        let a = Db::open(&t.path()).unwrap();
        assert_eq!(a.reset_stale().await.unwrap(), 0); // A is now a live worker
        let rec = a.add(new_rec("h1")).await.unwrap();
        a.enqueue(rec, "transcribe").await.unwrap();
        a.claim_next().await.unwrap().unwrap();

        let b = Db::open(&t.path()).unwrap();
        assert_eq!(b.reset_stale().await.unwrap(), 0);
        assert_eq!(b.list_for(rec).await.unwrap()[0].status, JobStatus::Running);

        // Both workers gone: a later start requeues the orphan.
        drop((a, b));
        let c = Db::open(&t.path()).unwrap();
        assert_eq!(c.reset_stale().await.unwrap(), 1);
        assert_eq!(c.list_for(rec).await.unwrap()[0].status, JobStatus::Queued);
    }

    #[tokio::test]
    async fn complete_on_requeued_job_does_not_enqueue_twice() {
        let t = TempDb::new("requeued");
        let db = Db::open(&t.path()).unwrap();
        let rec = db.add(new_rec("h1")).await.unwrap();
        db.enqueue(rec, "transcribe").await.unwrap();
        let stale = db.claim_next().await.unwrap().unwrap();
        db.reset_stale().await.unwrap(); // the job goes back to queued

        assert!(db.complete(&stale, Some("summarize")).await.is_err());
        assert!(db.fail(&stale, "late").await.is_err());
        let jobs = db.list_for(rec).await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].status, JobStatus::Queued);
    }

    /// A toggle aimed at an action that is no longer at that index is refused and changes nothing.
    #[tokio::test]
    async fn toggle_refuses_a_stale_action() {
        use liberarr_core::Error;
        use liberarr_core::summaries::{Action, MeetingSummary, SummaryRepo, ToggleAction};

        let t = TempDb::new("actions");
        let db = Db::open(&t.path()).unwrap();
        let id = db.add(new_rec("h1")).await.unwrap();
        let action = |text: &str| Action {
            action: text.into(),
            owner: None,
            due: None,
            start: None,
            done: false,
        };
        let summary = MeetingSummary {
            title: "t".into(),
            summary_md: "s".into(),
            mindmap: "mindmap\n  root".into(),
            actions: vec![action("call Bob"), action("send deck")],
        };
        SummaryRepo::save(&db, id, &summary).await.unwrap();
        let toggle = ToggleAction { summaries: &db };

        let s = toggle.run(id, 1, "send deck", true).await.unwrap();
        assert!(!s.actions[0].done && s.actions[1].done);

        let err = toggle.run(id, 0, "send deck", true).await.unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err:?}");
        let err = toggle.run(id, 2, "send deck", true).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
        let stored = SummaryRepo::get(&db, id).await.unwrap().unwrap();
        assert!(!stored.actions[0].done && stored.actions[1].done);
    }

    #[tokio::test]
    async fn counts_open_actions_over_all_recordings() {
        use liberarr_core::summaries::{Action, MeetingSummary, SummaryRepo};

        let t = TempDb::new("open_actions");
        let db = Db::open(&t.path()).unwrap();
        assert_eq!(db.open_actions().await.unwrap(), 0);
        let action = |text: &str, done| Action {
            action: text.into(),
            owner: None,
            due: None,
            start: None,
            done,
        };
        let summary = |actions| MeetingSummary {
            title: "t".into(),
            summary_md: "s".into(),
            mindmap: "mindmap\n  root".into(),
            actions,
        };
        let a = db.add(new_rec("h1")).await.unwrap();
        let b = db.add(new_rec("h2")).await.unwrap();
        db.add(new_rec("h3")).await.unwrap(); // no summary
        let both = summary(vec![
            action("a", true),
            action("b", false),
            action("c", false),
        ]);
        SummaryRepo::save(&db, a, &both).await.unwrap();
        SummaryRepo::save(
            &db,
            b,
            &summary(vec![action("d", false), action("e", true)]),
        )
        .await
        .unwrap();
        assert_eq!(db.open_actions().await.unwrap(), 3);
        assert!(db.set_action_done(a, 1, "b", true).await.unwrap());
        assert_eq!(db.open_actions().await.unwrap(), 2);
    }

    /// Issue #88: transcripts must not be readable by other local users.
    #[tokio::test]
    async fn db_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let side = |p: &Path, s: &str| {
            let mut o = p.as_os_str().to_owned();
            o.push(s);
            PathBuf::from(o)
        };
        let t = TempDb::new("perms");
        let path = t.path();

        let db = Db::open(&path).unwrap();
        db.add(new_rec("h1")).await.unwrap();
        for s in ["", "-wal", "-shm"] {
            assert_eq!(mode(&side(&path, s)), 0o600, "fresh {s}");
        }

        // Files created world-readable by an older version (`db` keeps the WAL alive, as a
        // crash would) are tightened on the next open.
        for s in ["", "-wal", "-shm"] {
            std::fs::set_permissions(side(&path, s), std::fs::Permissions::from_mode(0o644))
                .unwrap();
        }
        let db2 = Db::open(&path).unwrap();
        db2.add(new_rec("h2")).await.unwrap();
        for s in ["", "-wal", "-shm"] {
            assert_eq!(mode(&side(&path, s)), 0o600, "existing {s}");
        }
    }

    #[tokio::test]
    async fn ledger_roundtrip() {
        let t = TempDb::new("ledger");
        let db = Db::open(&t.path()).unwrap();
        let rec = db.add(new_rec("h1")).await.unwrap();
        assert!(db.known("dev-a").await.unwrap().is_empty());

        db.record("dev-a", 7, 1000, rec).await.unwrap();
        db.record("dev-a", 7, 1000, rec).await.unwrap(); // idempotent
        db.record("dev-a", 9, 2000, rec).await.unwrap();

        assert_eq!(db.known("dev-a").await.unwrap(), [7, 9].into());
        assert!(db.known("dev-b").await.unwrap().is_empty());
    }
}
