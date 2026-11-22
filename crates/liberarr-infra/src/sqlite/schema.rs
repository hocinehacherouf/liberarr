//! Schema migrations, tracked with `PRAGMA user_version`: `MIGRATIONS[i]` takes version i to i+1.
use super::storage;
use liberarr_core::{Error, Result};
use rusqlite::{Connection, TransactionBehavior};

pub(super) const MIGRATIONS: &[&str] = &[
    "
CREATE TABLE recordings (
    id          INTEGER PRIMARY KEY,
    title       TEXT,
    source      TEXT NOT NULL,
    source_ref  TEXT NOT NULL UNIQUE,
    audio_path  TEXT NOT NULL,
    duration_s  REAL,
    created_at  TEXT NOT NULL,
    export_path TEXT
);
CREATE TABLE transcripts (
    recording_id   INTEGER PRIMARY KEY REFERENCES recordings(id),
    engine         TEXT NOT NULL,
    language       TEXT,
    text           TEXT NOT NULL,
    segments       TEXT NOT NULL,
    has_timestamps INTEGER NOT NULL
);
CREATE TABLE summaries (
    recording_id INTEGER PRIMARY KEY REFERENCES recordings(id),
    title        TEXT NOT NULL,
    summary_md   TEXT NOT NULL,
    mindmap      TEXT NOT NULL,
    actions      TEXT NOT NULL
);
CREATE TABLE jobs (
    id           INTEGER PRIMARY KEY,
    recording_id INTEGER NOT NULL REFERENCES recordings(id),
    step         TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'queued',
    attempts     INTEGER NOT NULL DEFAULT 0,
    error        TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX jobs_by_status ON jobs(status, id);
CREATE INDEX jobs_by_recording ON jobs(recording_id, id);
CREATE TABLE device_sessions (
    serial_hash  TEXT NOT NULL,
    session_id   INTEGER NOT NULL,
    size         INTEGER NOT NULL,
    recording_id INTEGER NOT NULL REFERENCES recordings(id),
    pulled_at    TEXT NOT NULL,
    PRIMARY KEY (serial_hash, session_id)
);
",
    "
CREATE TABLE diarizations (
    recording_id INTEGER PRIMARY KEY REFERENCES recordings(id),
    engine       TEXT NOT NULL,
    turns        TEXT NOT NULL
);
CREATE TABLE speaker_settings (
    recording_id        INTEGER PRIMARY KEY REFERENCES recordings(id),
    requested_speakers  INTEGER
);
CREATE TABLE speaker_names (
    recording_id INTEGER NOT NULL REFERENCES recordings(id),
    speaker      INTEGER NOT NULL,
    name         TEXT NOT NULL,
    PRIMARY KEY (recording_id, speaker)
);
",
];

/// Applies the pending migrations in one `BEGIN IMMEDIATE` transaction. The version is re-read
/// inside it, so two processes opening the same file never both apply a migration.
/// Refuses a database written by a newer liberarr instead of running on a schema it does not know.
pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let version: i64 = tx
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(storage)?;
    if version > MIGRATIONS.len() as i64 {
        return Err(Error::Storage(
            "database created by a newer liberarr".into(),
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        tx.execute_batch(sql).map_err(storage)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)
            .map_err(storage)?;
    }
    tx.commit().map_err(storage)
}
