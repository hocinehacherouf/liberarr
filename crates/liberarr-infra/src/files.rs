//! Filesystem adapters: atomic note writes, device-pull scratch space, audio storage and splitting.
//! Files holding recordings or keys are created 0600, their directories 0700.
use liberarr_core::audio::ogg::{opus_duration_seconds, split_ogg};
use liberarr_core::device::ports::PartialStore;
use liberarr_core::exports::NoteStore;
use liberarr_core::recordings::{AudioStore, StoredAudio};
use liberarr_core::transcription::AudioSplitter;
use liberarr_core::{Error, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

fn io_err(what: &str, e: io::Error) -> Error {
    // Paths and content stay out of the message: they can name a meeting.
    Error::Storage(format!("{what}: {}", e.kind()))
}

fn mkdir(dir: &Path, mode: u32) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(mode)
        .create(dir)
        .map_err(|e| io_err("create dir", e))
}

/// Writes `<path>.tmp` then renames over `path`; the tmp is removed on any error, so an
/// existing file is never lost or half-written.
fn write_atomic(
    path: &Path,
    mode: u32,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    let tmp = PathBuf::from(name);
    let res = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(&tmp)
        .and_then(|mut f| fill(&mut f).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, path));
    if res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    res
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        mkdir(dir, 0o700)?;
    }
    write_atomic(path, 0o600, |f| f.write_all(data)).map_err(|e| io_err("write file", e))
}

/// SHA-256 (hex) of a file. A read error is an error, never a hash of partial content.
pub fn sha256_file(p: &Path) -> Result<String> {
    let mut f = File::open(p).map_err(|e| io_err("open audio", e))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io_err("read audio", e)),
        }
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn opus_duration_from_file(path: &Path) -> Result<Option<f64>> {
    let mut file = File::open(path).map_err(|e| io_err("open audio", e))?;
    let mut prefix = [0; 4];
    match file.read_exact(&mut prefix) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(io_err("read audio", e)),
    }
    if &prefix != b"OggS" {
        return Ok(None);
    }
    let mut data = prefix.to_vec();
    file.read_to_end(&mut data)
        .map_err(|e| io_err("read audio", e))?;
    Ok(opus_duration_seconds(&data))
}

/// `read` gives `None` only for a missing note; any other error is returned, so the owner's
/// text is never mistaken for "no note" and overwritten.
pub struct FsNoteStore;

impl NoteStore for FsNoteStore {
    fn read(&self, path: &Path) -> Result<Option<String>> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err("read note", e)),
        }
    }

    fn write_atomic(&self, path: &Path, text: &str) -> Result<()> {
        if let Some(dir) = path.parent() {
            mkdir(dir, 0o755)?;
        }
        write_atomic(path, 0o644, |f| f.write_all(text.as_bytes()))
            .map_err(|e| io_err("write note", e))
    }
}

/// `dir` = `<home>/device/partial`, `raw` = `<home>/device/raw`, `audio` = `<home>/audio`.
/// Keys are `<serial_hash>-<session>`, never the serial.
pub struct FsPartialStore {
    pub dir: PathBuf,
    pub raw: PathBuf,
    pub audio: PathBuf,
}

impl FsPartialStore {
    fn part(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.part"))
    }
    fn staged(&self, key: &str) -> PathBuf {
        self.audio.join(format!("device-{key}.ogg"))
    }
}

fn absent_ok(r: io::Result<()>) -> Result<()> {
    match r {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err("remove file", e)),
        _ => Ok(()),
    }
}

impl PartialStore for FsPartialStore {
    fn len(&self, key: &str) -> Result<u64> {
        match fs::metadata(self.part(key)) {
            Ok(m) => Ok(m.len()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(io_err("stat part", e)),
        }
    }

    fn append(&self, key: &str, data: &[u8]) -> Result<()> {
        mkdir(&self.dir, 0o700)?;
        OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(self.part(key))
            .and_then(|mut f| f.write_all(data))
            .map_err(|e| io_err("append part", e))
    }

    fn take(&self, key: &str) -> Result<Vec<u8>> {
        match fs::read(self.part(key)) {
            Ok(data) => Ok(data),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io_err("read part", e)),
        }
    }

    /// Also drops an unconsumed staged Ogg (an `Ingest` dedup hit leaves it behind).
    fn discard(&self, key: &str) -> Result<()> {
        absent_ok(fs::remove_file(self.part(key)))?;
        absent_ok(fs::remove_file(self.staged(key)))
    }

    fn keep_raw(&self, key: &str, data: &[u8]) -> Result<PathBuf> {
        let path = self.raw.join(format!("{key}.raw"));
        write_private(&path, data)?;
        Ok(path)
    }

    fn stage_audio(&self, key: &str, ogg: &[u8]) -> Result<PathBuf> {
        let path = self.staged(key);
        write_private(&path, ogg)?;
        Ok(path)
    }
}

/// Stores audio as `<audio>/<fingerprint>.<ext>`. Only a regular (non-symlink) `device-*` file
/// directly in `audio` is a staged device Ogg Liberarr wrote itself: it is moved. Anything
/// else is the user's own file: it is copied, never moved or deleted.
pub struct FsAudioStore {
    pub audio: PathBuf,
}

impl AudioStore for FsAudioStore {
    fn fingerprint(&self, src: &Path) -> Result<String> {
        sha256_file(src)
    }

    fn store(&self, src: &Path, fingerprint: &str) -> Result<StoredAudio> {
        let duration_s = opus_duration_from_file(src)?;
        mkdir(&self.audio, 0o700)?;
        let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("ogg");
        let dest = self.audio.join(format!("{fingerprint}.{ext}"));
        let staged = src.parent() == Some(self.audio.as_path())
            && src
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("device-"))
            && fs::symlink_metadata(src).is_ok_and(|m| m.file_type().is_file());
        if staged {
            // Same directory as the staging area, so a plain rename (never cross-device).
            fs::rename(src, &dest).map_err(|e| io_err("move audio", e))?;
        } else {
            let mut from = File::open(src).map_err(|e| io_err("read audio", e))?;
            write_atomic(&dest, 0o600, |to| io::copy(&mut from, to).map(drop))
                .map_err(|e| io_err("copy audio", e))?;
        }
        Ok(StoredAudio {
            path: dest,
            duration_s,
        })
    }
}

/// Reads the file whole: recordings are tens of MB at most.
pub struct OggSplitter;

impl AudioSplitter for OggSplitter {
    fn split(&self, path: &Path, max_bytes: usize) -> Result<Vec<(Vec<u8>, f64)>> {
        let data = fs::read(path)
            .map_err(|e| Error::Storage(format!("audio unreadable: {}", e.kind())))?;
        split_ogg(&data, max_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Unique temp dir, removed on drop (no extra dev-dependency).
    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Self {
            let p =
                std::env::temp_dir().join(format!("liberarr-files-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).expect("temp dir");
            Self(p)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            // A test may leave a read-only dir behind.
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn opus_file() -> Vec<u8> {
        fn page(seq: u32, granule: i64, flags: u8, body: &[u8]) -> Vec<u8> {
            let mut page = b"OggS".to_vec();
            page.extend_from_slice(&[0, flags]);
            page.extend_from_slice(&granule.to_le_bytes());
            page.extend_from_slice(&1234u32.to_le_bytes());
            page.extend_from_slice(&seq.to_le_bytes());
            page.extend_from_slice(&[0; 4]);
            page.extend_from_slice(&[1, body.len() as u8]);
            page.extend_from_slice(body);

            let crc = page.iter().enumerate().fold(0u32, |crc, (i, &byte)| {
                let byte = if (22..26).contains(&i) { 0 } else { byte };
                let mut crc = crc ^ ((byte as u32) << 24);
                for _ in 0..8 {
                    crc = if crc & 0x8000_0000 != 0 {
                        (crc << 1) ^ 0x04C1_1DB7
                    } else {
                        crc << 1
                    };
                }
                crc
            });
            page[22..26].copy_from_slice(&crc.to_le_bytes());
            page
        }

        let mut head = b"OpusHead\x01\x01".to_vec();
        head.extend_from_slice(&312u16.to_le_bytes());
        head.extend_from_slice(&[0; 7]);
        [
            page(0, 0, 0x02, &head),
            page(1, -1, 0, b"OpusTags"),
            page(2, 48_312, 0x04, b"audio packet"),
        ]
        .concat()
    }

    #[test]
    fn failed_write_keeps_existing_note() {
        let d = Dir::new("note");
        let note = d.0.join("vault/n.md");
        FsNoteStore.write_atomic(&note, "owner text").unwrap();
        std::fs::set_permissions(
            note.parent().unwrap(),
            std::fs::Permissions::from_mode(0o555),
        )
        .unwrap();
        assert!(FsNoteStore.write_atomic(&note, "new").is_err());
        assert_eq!(
            FsNoteStore.read(&note).unwrap().as_deref(),
            Some("owner text")
        );
        assert!(!note.with_extension("md.tmp").exists());
        assert_eq!(FsNoteStore.read(&d.0.join("vault/none.md")).unwrap(), None);
    }

    #[test]
    fn partial_append_take_roundtrip() {
        let d = Dir::new("partial");
        let s = FsPartialStore {
            dir: d.0.join("device/partial"),
            raw: d.0.join("device/raw"),
            audio: d.0.join("audio"),
        };
        assert_eq!(s.len("h-1").unwrap(), 0);
        assert!(
            s.take("h-1").unwrap().is_empty(),
            "a missing part reads as empty"
        );
        s.append("h-1", b"abc").unwrap();
        s.append("h-1", b"de").unwrap();
        assert_eq!(s.len("h-1").unwrap(), 5);
        assert_eq!(s.take("h-1").unwrap(), b"abcde");
        // `take` keeps the part until `discard`.
        assert_eq!(s.len("h-1").unwrap(), 5);
        let staged = s.stage_audio("h-1", b"ogg").unwrap();
        s.discard("h-1").unwrap();
        assert_eq!(s.len("h-1").unwrap(), 0);
        assert!(!staged.exists(), "discard drops an unconsumed staged copy");
        s.discard("h-1").unwrap();
        let raw = s.keep_raw("h-1", b"enc").unwrap();
        assert_eq!(std::fs::read(&raw).unwrap(), b"enc");
        assert_eq!(
            std::fs::metadata(&raw).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn process_input_is_copied_not_moved() {
        let d = Dir::new("audio");
        let store = FsAudioStore {
            audio: d.0.join("audio"),
        };
        let partials = FsPartialStore {
            dir: d.0.join("p"),
            raw: d.0.join("r"),
            audio: store.audio.clone(),
        };
        let user = d.0.join("meeting.ogg");
        std::fs::write(&user, b"user audio").unwrap();
        let fp = store.fingerprint(&user).unwrap();
        let kept = store.store(&user, &fp).unwrap();
        assert!(user.exists(), "the user's original must survive");
        assert_eq!(std::fs::read(&kept.path).unwrap(), b"user audio");
        assert_eq!(kept.duration_s, None);

        // A staged device Ogg is moved: no second copy is left in staging.
        let staged = partials.stage_audio("h-2", b"device ogg").unwrap();
        let fp2 = store.fingerprint(&staged).unwrap();
        let kept2 = store.store(&staged, &fp2).unwrap();
        assert!(!staged.exists());
        assert_eq!(std::fs::read(&kept2.path).unwrap(), b"device ogg");
        assert_eq!(
            std::fs::metadata(&kept2.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn store_records_duration_for_opus_and_keeps_the_user_file() {
        let d = Dir::new("opus-duration");
        let audio = d.0.join("audio");
        let store = FsAudioStore {
            audio: audio.clone(),
        };
        let user = d.0.join("meeting.ogg");
        let data = opus_file();
        std::fs::write(&user, &data).unwrap();

        let fp = store.fingerprint(&user).unwrap();
        let stored = store.store(&user, &fp).unwrap();

        assert_eq!(stored.duration_s, Some(1.0));
        assert_eq!(std::fs::read(&stored.path).unwrap(), data);
        assert!(user.exists(), "importing must not remove the user's file");
    }

    #[test]
    fn user_file_inside_audio_dir_is_copied() {
        let d = Dir::new("inside");
        let audio = d.0.join("audio");
        std::fs::create_dir_all(&audio).unwrap();
        let store = FsAudioStore {
            audio: audio.clone(),
        };
        let user = audio.join("mine.ogg");
        std::fs::write(&user, b"mine").unwrap();
        let kept = store.store(&user, "fp1").unwrap();
        assert!(user.exists(), "non-device file in the audio dir must stay");
        assert_eq!(std::fs::read(&kept.path).unwrap(), b"mine");

        // `..` escapes and symlinks named `device-*` are copied too.
        let outside = d.0.join("a.ogg");
        std::fs::write(&outside, b"outside").unwrap();
        let sneaky = audio.join("../a.ogg");
        store.store(&sneaky, "fp2").unwrap();
        assert!(outside.exists());
        let link = audio.join("device-link.ogg");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        store.store(&link, "fp3").unwrap();
        assert!(link.exists() && outside.exists());
    }
}
