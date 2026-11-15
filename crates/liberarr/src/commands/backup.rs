//! `liberarr backup <dest>`: a consistent copy of the home, safe while a worker runs, without
//! the `.env`. It is built in `<dest>.incomplete` and renamed to `<dest>` only once complete, so
//! a failed or interrupted backup never looks like a good one.
use super::Outcome;
use crate::config::{self, is_unset, parse_env};
use liberarr_infra::backup::backup_home;
use rust_i18n::t;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub fn run(dest: &Path) -> Outcome {
    let home = config::home()?;
    let home = fs::canonicalize(&home)
        .map_err(|e| t!("backup.no_home", path = home.display(), kind = e.kind()).into_owned())?;
    let dest = target(&home, dest)?;
    let mut name = dest.file_name().unwrap_or_default().to_owned();
    name.push(".incomplete");
    let work = dest.with_file_name(name);
    if work.starts_with(&home) {
        return Err(t!("backup.inside_home", home = home.display()).into());
    }
    match fs::DirBuilder::new().mode(0o700).create(&work) {
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            return Err(t!("backup.leftover", path = work.display()).into());
        }
        r => r.map_err(|e| t!("backup.cannot_create", kind = e.kind()).into_owned())?,
    }
    // An existing empty `dest` is replaced by the rename (POSIX allows it for an empty directory).
    let copied = match backup_home(&home, &work).and_then(|c| {
        fs::rename(&work, &dest)
            .map(|()| c)
            .map_err(|e| liberarr_core::Error::Storage(format!("rename: {}", e.kind())))
    }) {
        Ok(c) => c,
        Err(e) => {
            // Everything in `work` was written by this run: removing it loses nothing.
            let key = match fs::remove_dir_all(&work) {
                Ok(()) => "backup.failed_removed",
                Err(_) => "backup.failed_left",
            };
            eprintln!("{}", t!(key, path = work.display()));
            return Err(e.into());
        }
    };
    println!(
        "{}",
        t!(
            "backup.done",
            path = dest.display(),
            db = mb(copied.db_bytes),
            files = copied.files,
            size = mb(copied.bytes)
        )
    );
    if copied.skipped > 0 {
        println!("{}", t!("backup.skipped", count = copied.skipped));
    }
    if let Some(notes) = notes_outside(&home) {
        println!("{}", t!("backup.notes_outside", path = notes.display()));
    }
    println!("{}", t!("backup.env_not_included"));
    Ok(ExitCode::SUCCESS)
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// `dest` as an absolute path with symbolic links resolved, refused when it is the home or
/// inside it (the backup would copy itself), or when it exists and is not an empty directory.
fn target(home: &Path, dest: &Path) -> Result<PathBuf, String> {
    let dest = std::path::absolute(dest).map_err(|e| e.kind().to_string())?;
    let resolved = match fs::canonicalize(&dest) {
        Ok(p) => p,
        // Does not exist yet: resolve its parent, which must exist.
        Err(e) if e.kind() == ErrorKind::NotFound => match (dest.parent(), dest.file_name()) {
            (Some(parent), Some(name)) => fs::canonicalize(parent)
                .map(|p| p.join(name))
                .map_err(|_| t!("backup.no_parent", path = parent.display()).into_owned())?,
            _ => return Err(t!("backup.no_parent", path = dest.display()).into_owned()),
        },
        Err(e) => return Err(e.kind().to_string()),
    };
    if resolved.starts_with(home) {
        return Err(t!("backup.inside_home", home = home.display()).into_owned());
    }
    if let Ok(mut entries) = fs::read_dir(&resolved) {
        if entries.next().is_some() {
            return Err(t!("backup.not_empty", path = resolved.display()).into_owned());
        }
    } else if fs::symlink_metadata(&resolved).is_ok() {
        return Err(t!("backup.not_empty", path = resolved.display()).into_owned());
    }
    Ok(resolved)
}

/// `LIBERARR_OBSIDIAN_DIR` when it points outside the home: those notes are not in the backup.
/// Only that line of the `.env` is used; a missing `.env` means the default (inside the home).
fn notes_outside(home: &Path) -> Option<PathBuf> {
    let env = parse_env(&fs::read_to_string(home.join(".env")).ok()?);
    let dir = PathBuf::from(env.get("LIBERARR_OBSIDIAN_DIR").filter(|v| !is_unset(v))?);
    let real = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
    (!real.starts_with(home)).then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backup into the home would copy itself; one into a folder in use would mix with it.
    #[test]
    fn refuses_a_destination_in_the_home_or_not_empty() {
        let root =
            std::env::temp_dir().join(format!("liberarr-backup-dest-{}", std::process::id()));
        let home = root.join("home");
        fs::create_dir_all(home.join("audio")).unwrap();
        fs::create_dir_all(root.join("full")).unwrap();
        fs::write(root.join("full/x"), "").unwrap();
        std::os::unix::fs::symlink(&home, root.join("link")).unwrap();
        let home = fs::canonicalize(&home).unwrap();

        for bad in [
            home.clone(),
            home.join("backup"),
            home.join("audio"),
            home.join("audio/../backup"),
            root.join("link/backup"),
            root.join("full"),
        ] {
            assert!(target(&home, &bad).is_err(), "{bad:?}");
        }
        assert!(target(&home, &root.join("new")).is_ok());
        assert!(target(&home, &root.join("missing/new")).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
