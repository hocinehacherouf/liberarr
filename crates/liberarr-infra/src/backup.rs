//! `liberarr backup`: a consistent copy of a home that is itself a home, without `.env`.
//! The database is copied first (SQLite `VACUUM INTO`), then the files: audio is stored before
//! its row, so every recording in the copy finds its audio.
use crate::sqlite::Db;
use liberarr_core::{Error, Result};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

/// What went into the backup: counts and sizes only.
#[derive(Default)]
pub struct Copied {
    pub db_bytes: u64,
    pub files: u64,
    pub bytes: u64,
    /// Symbolic links and special files: never followed, never copied.
    pub skipped: u64,
}

fn io_err(what: &str, e: io::Error) -> Error {
    // Paths stay out of the message: an audio or note name can name a meeting.
    Error::Storage(format!("{what}: {}", e.kind()))
}

/// Copies `home` into `dest`, an existing empty directory outside `home`. Leaves out `.env*`
/// (secrets), the live database files, `*.tmp` and unfinished downloads (`device/partial`).
pub fn backup_home(home: &Path, dest: &Path) -> Result<Copied> {
    let db_path = home.join("liberarr.db");
    if !db_path.is_file() {
        return Err(Error::Storage("no database in the home".into()));
    }
    let snapshot = dest.join("liberarr.db");
    Db::open(&db_path)?.snapshot(&snapshot)?;
    let mut n = Copied {
        db_bytes: fs::metadata(&snapshot)
            .map_err(|e| io_err("stat database copy", e))?
            .len(),
        ..Copied::default()
    };
    copy_tree(home, home, dest, &mut n)?;
    Ok(n)
}

fn left_out(home: &Path, dir: &Path, name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    let top = dir == home;
    name.ends_with(".tmp")
        || (top && (name.starts_with(".env") || name.starts_with("liberarr.db")))
        || (dir == home.join("device") && name == "partial")
}

fn copy_tree(home: &Path, from: &Path, to: &Path, n: &mut Copied) -> Result<()> {
    for entry in fs::read_dir(from).map_err(|e| io_err("read dir", e))? {
        let entry = entry.map_err(|e| io_err("read dir", e))?;
        if left_out(home, from, &entry.file_name()) {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        // Not followed: a link could lead out of the home.
        let kind = entry.file_type().map_err(|e| io_err("stat", e))?;
        if kind.is_dir() {
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&dst)
                .map_err(|e| io_err("create dir", e))?;
            copy_tree(home, &src, &dst, n)?;
        } else if kind.is_file() {
            let mut from = match File::open(&src) {
                Ok(f) => f,
                // Renamed or removed by a running worker since the listing (a staged download).
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_err("open file", e)),
            };
            let mut to = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&dst)
                .map_err(|e| io_err("create file", e))?;
            n.bytes += io::copy(&mut from, &mut to).map_err(|e| io_err("copy file", e))?;
            to.sync_all().map_err(|e| io_err("sync file", e))?;
            n.files += 1;
        } else {
            n.skipped += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::FsAudioStore;
    use liberarr_core::recordings::{AudioStore, NewRecording, RecordingRepo, Source};
    use liberarr_core::transcription::{Segment, Transcript, TranscriptRepo};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn modes(dir: &Path, out: &mut Vec<(PathBuf, u32)>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            out.push((
                p.clone(),
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            ));
            if p.is_dir() {
                modes(&p, out);
            }
        }
    }

    /// The backup is taken while another connection keeps adding recordings (audio, then row,
    /// then a transcript), with its commits still in the `-wal`. The copy
    /// must open as a home, with every recording's audio byte for byte, and no secret.
    #[tokio::test(flavor = "multi_thread")]
    async fn backup_during_writes_restores_to_a_working_home() {
        let root = Dir(std::env::temp_dir().join(format!(
            "liberarr-backup-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_nanosecond()
        )));
        let (home, dest) = (root.0.join("home"), root.0.join("backup"));
        fs::create_dir_all(home.join("device/partial")).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(home.join(".env"), "LIBERARR_SECRET_KEY=fake\n").unwrap();
        fs::write(home.join("device/keys.bin"), b"sealed").unwrap();
        fs::write(home.join("device/partial/x.part"), b"half").unwrap();
        std::os::unix::fs::symlink("/etc", home.join("outside")).unwrap();

        let db = Db::open(&home.join("liberarr.db")).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let added = Arc::new(AtomicUsize::new(0));
        let writer = tokio::spawn({
            let (home, src) = (home.clone(), root.0.join("in.ogg"));
            let (stop, added) = (stop.clone(), added.clone());
            async move {
                let store = FsAudioStore {
                    audio: home.join("audio"),
                };
                let mut i = 0;
                while !stop.load(Ordering::Relaxed) {
                    fs::write(&src, format!("audio {i}")).unwrap();
                    let stored = store.store(&src, &format!("fp{i}")).unwrap();
                    let id = db
                        .add(NewRecording {
                            title: None,
                            source: Source::File,
                            source_ref: format!("ref{i}"),
                            audio_path: stored.path,
                            duration_s: None,
                            created_at: "2026-03-01T10:00:00Z".parse().unwrap(),
                        })
                        .await
                        .unwrap();
                    let text = "salut ".repeat(200);
                    let t = Transcript {
                        engine: "e".into(),
                        language: None,
                        text: text.clone(),
                        segments: vec![Segment {
                            start: 0.0,
                            end: 1.0,
                            text,
                        }],
                        has_timestamps: true,
                    };
                    TranscriptRepo::save(&db, id, &t).await.unwrap();
                    i += 1;
                    added.store(i, Ordering::Relaxed);
                }
            }
        });
        while added.load(Ordering::Relaxed) < 20 {
            tokio::task::yield_now().await;
        }
        let before = added.load(Ordering::Relaxed);
        let (h, d) = (home.clone(), dest.clone());
        let copied = tokio::task::spawn_blocking(move || backup_home(&h, &d))
            .await
            .unwrap()
            .unwrap();
        let during = added.load(Ordering::Relaxed) - before;
        stop.store(true, Ordering::Relaxed);
        writer.await.unwrap();
        assert!(during > 0, "the writer must run while the backup is taken");
        assert_eq!(copied.skipped, 1, "the symbolic link is not followed");

        assert!(!dest.join(".env").exists());
        assert!(!dest.join("liberarr.db-wal").exists());
        assert!(!dest.join("device/partial").exists());
        assert!(!dest.join("outside").exists());
        assert_eq!(fs::read(dest.join("device/keys.bin")).unwrap(), b"sealed");
        let mut all = Vec::new();
        modes(&dest, &mut all);
        for (p, mode) in all {
            let want = if p.is_dir() { 0o700 } else { 0o600 };
            assert_eq!(mode, want, "{p:?}");
        }

        let restored = Db::open(&dest.join("liberarr.db")).unwrap();
        let items = restored.list_items().await.unwrap();
        assert!(items.len() >= before, "{} < {before}", items.len());
        for item in items {
            let rec = RecordingRepo::get(&restored, item.id)
                .await
                .unwrap()
                .unwrap();
            assert!(rec.audio_path.starts_with(&dest), "{:?}", rec.audio_path);
            let n = rec.source_ref.trim_start_matches("ref");
            assert_eq!(
                fs::read(&rec.audio_path).unwrap(),
                format!("audio {n}").as_bytes()
            );
        }
    }
}
