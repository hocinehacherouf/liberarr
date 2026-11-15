//! Recording, transcript, summary and device-ledger ports on SQLite.
use super::{Db, now, parse_ts, storage};
use liberarr_core::device::ports::DeviceLedger;
use liberarr_core::recordings::{
    NewRecording, Recording, RecordingItem, RecordingRepo, RecordingStatus, Source,
};
use liberarr_core::summaries::{MeetingSummary, SummaryRepo};
use liberarr_core::transcription::{Segment, Transcript, TranscriptRepo};
use liberarr_core::{Error, Result};
use rusqlite::types::Type;
use rusqlite::{OptionalExtension, params};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Fixed message: serde errors can quote the stored (private) content.
fn corrupt_json(col: usize) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(col, Type::Text, "invalid JSON in column".into())
}

fn parse_source(col: usize, s: &str) -> rusqlite::Result<Source> {
    match s {
        "device" => Ok(Source::Device),
        "file" => Ok(Source::File),
        _ => Err(rusqlite::Error::InvalidColumnType(
            col,
            "source".into(),
            Type::Text,
        )),
    }
}

fn path_str(p: &Path) -> Result<String> {
    p.to_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::Storage("path is not valid UTF-8".into()))
}

impl RecordingRepo for Db {
    async fn get(&self, id: i64) -> Result<Option<Recording>> {
        let audio_dir = self.audio_dir.clone();
        self.run(move |c| {
            c.query_row(
                "SELECT id, title, source, source_ref, audio_path, duration_s, created_at, export_path
                 FROM recordings WHERE id = ?1",
                [id],
                |r| {
                    Ok(Recording {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        source: parse_source(2, &r.get::<_, String>(2)?)?,
                        source_ref: r.get(3)?,
                        // `join` keeps a legacy absolute value as it is.
                        audio_path: audio_dir.join(r.get::<_, String>(4)?),
                        duration_s: r.get(5)?,
                        created_at: parse_ts(6, &r.get::<_, String>(6)?)?,
                        export_path: r.get::<_, Option<String>>(7)?.map(PathBuf::from),
                    })
                },
            )
            .optional()
        })
        .await
    }

    async fn find_by_ref(&self, source_ref: &str) -> Result<Option<i64>> {
        let source_ref = source_ref.to_owned();
        self.run(move |c| {
            c.query_row(
                "SELECT id FROM recordings WHERE source_ref = ?1",
                [source_ref],
                |r| r.get(0),
            )
            .optional()
        })
        .await
    }

    async fn add(&self, new: NewRecording) -> Result<i64> {
        let audio_path = path_str(
            new.audio_path
                .strip_prefix(&self.audio_dir)
                .unwrap_or(&new.audio_path),
        )?;
        self.run(move |c| {
            c.execute(
                "INSERT INTO recordings (title, source, source_ref, audio_path, duration_s, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    new.title,
                    new.source.as_str(),
                    new.source_ref,
                    audio_path,
                    new.duration_s,
                    new.created_at.to_string()
                ],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
    }

    async fn set_export_path(&self, id: i64, path: &Path) -> Result<()> {
        let path = path_str(path)?;
        let changed = self
            .run(move |c| {
                c.execute(
                    "UPDATE recordings SET export_path = ?2 WHERE id = ?1",
                    params![id, path],
                )
            })
            .await?;
        if changed == 0 {
            return Err(Error::NotFound("recording".into()));
        }
        Ok(())
    }

    /// Latest job decides the status; the summary title wins over the recording title.
    /// "No speech" is derived from the stored transcript, not stored on its own.
    async fn list_items(&self) -> Result<Vec<RecordingItem>> {
        self.run(|c| {
            let mut stmt = c.prepare(
                "SELECT r.id, COALESCE(s.title, r.title), r.source, r.duration_s, r.created_at,
                        j.status, CASE WHEN j.status = 'failed' THEN j.step END,
                        COALESCE(trim(t.text, char(32, 9, 10, 13)) = '', 0)
                 FROM recordings r
                 LEFT JOIN summaries s ON s.recording_id = r.id
                 LEFT JOIN transcripts t ON t.recording_id = r.id
                 LEFT JOIN jobs j ON j.id = (SELECT MAX(id) FROM jobs WHERE recording_id = r.id)
                 ORDER BY r.created_at DESC, r.id DESC",
            )?;
            stmt.query_map([], |r| {
                let status = match r.get::<_, Option<String>>(5)?.as_deref() {
                    Some("queued" | "running") => RecordingStatus::Processing,
                    Some("failed") => RecordingStatus::Failed,
                    _ => RecordingStatus::Done,
                };
                Ok(RecordingItem {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    source: parse_source(2, &r.get::<_, String>(2)?)?,
                    duration_s: r.get(3)?,
                    created_at: parse_ts(4, &r.get::<_, String>(4)?)?,
                    failed_step: r.get(6)?,
                    no_speech: status == RecordingStatus::Done && r.get(7)?,
                    status,
                })
            })?
            .collect()
        })
        .await
    }
}

impl TranscriptRepo for Db {
    async fn save(&self, recording_id: i64, t: &Transcript) -> Result<()> {
        // Segments as `[start, end, text]` triples.
        let segments = serde_json::to_string(
            &t.segments
                .iter()
                .map(|s| (s.start, s.end, &s.text))
                .collect::<Vec<_>>(),
        )
        .map_err(storage)?;
        let t = t.clone();
        self.run(move |c| {
            c.execute(
                "INSERT INTO transcripts (recording_id, engine, language, text, segments, has_timestamps)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(recording_id) DO UPDATE SET engine = excluded.engine,
                   language = excluded.language, text = excluded.text,
                   segments = excluded.segments, has_timestamps = excluded.has_timestamps",
                params![recording_id, t.engine, t.language, t.text, segments, t.has_timestamps],
            )?;
            Ok(())
        })
        .await
    }

    async fn get(&self, recording_id: i64) -> Result<Option<Transcript>> {
        self.run(move |c| {
            c.query_row(
                "SELECT engine, language, text, segments, has_timestamps FROM transcripts
                 WHERE recording_id = ?1",
                [recording_id],
                |r| {
                    let segments: Vec<(f64, f64, String)> =
                        serde_json::from_str(&r.get::<_, String>(3)?)
                            .map_err(|_| corrupt_json(3))?;
                    Ok(Transcript {
                        engine: r.get(0)?,
                        language: r.get(1)?,
                        text: r.get(2)?,
                        segments: segments
                            .into_iter()
                            .map(|(start, end, text)| Segment { start, end, text })
                            .collect(),
                        has_timestamps: r.get(4)?,
                    })
                },
            )
            .optional()
        })
        .await
    }
}

impl SummaryRepo for Db {
    async fn save(&self, recording_id: i64, s: &MeetingSummary) -> Result<()> {
        let actions = serde_json::to_string(&s.actions).map_err(storage)?;
        let s = s.clone();
        self.run(move |c| {
            c.execute(
                "INSERT INTO summaries (recording_id, title, summary_md, mindmap, actions)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(recording_id) DO UPDATE SET title = excluded.title,
                   summary_md = excluded.summary_md, mindmap = excluded.mindmap,
                   actions = excluded.actions",
                params![recording_id, s.title, s.summary_md, s.mindmap, actions],
            )?;
            Ok(())
        })
        .await
    }

    async fn get(&self, recording_id: i64) -> Result<Option<MeetingSummary>> {
        self.run(move |c| {
            c.query_row(
                "SELECT title, summary_md, mindmap, actions FROM summaries WHERE recording_id = ?1",
                [recording_id],
                |r| {
                    Ok(MeetingSummary {
                        title: r.get(0)?,
                        summary_md: r.get(1)?,
                        mindmap: r.get(2)?,
                        actions: serde_json::from_str(&r.get::<_, String>(3)?)
                            .map_err(|_| corrupt_json(3))?,
                    })
                },
            )
            .optional()
        })
        .await
    }

    async fn set_action_done(
        &self,
        recording_id: i64,
        index: usize,
        action: &str,
        done: bool,
    ) -> Result<bool> {
        let Ok(index) = i64::try_from(index) else {
            return Ok(false);
        };
        let action = action.to_owned();
        self.run(move |c| {
            // One statement: a concurrent toggle or summary save cannot interleave with it.
            let n = c.execute(
                "UPDATE summaries
                 SET actions = json_set(actions, '$[' || ?2 || '].done', json(?3))
                 WHERE recording_id = ?1 AND json_extract(actions, '$[' || ?2 || '].action') = ?4",
                params![
                    recording_id,
                    index,
                    if done { "true" } else { "false" },
                    action
                ],
            )?;
            Ok(n == 1)
        })
        .await
    }

    async fn open_actions(&self) -> Result<u32> {
        // `done` is absent or false on an open action, so `IS NOT 1` counts both.
        self.run(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM summaries s, json_each(s.actions) a
                 WHERE json_extract(a.value, '$.done') IS NOT 1",
                [],
                |r| r.get(0),
            )
        })
        .await
    }
}

impl DeviceLedger for Db {
    async fn known(&self, serial_hash: &str) -> Result<HashSet<u32>> {
        let serial_hash = serial_hash.to_owned();
        self.run(move |c| {
            let mut stmt =
                c.prepare("SELECT session_id FROM device_sessions WHERE serial_hash = ?1")?;
            stmt.query_map([serial_hash], |r| r.get(0))?.collect()
        })
        .await
    }

    async fn record(
        &self,
        serial_hash: &str,
        session: u32,
        size: u32,
        recording_id: i64,
    ) -> Result<()> {
        let serial_hash = serial_hash.to_owned();
        self.run(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO device_sessions
                   (serial_hash, session_id, size, recording_id, pulled_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![serial_hash, session, size, recording_id, now()],
            )?;
            Ok(())
        })
        .await
    }
}

impl Db {
    /// When a device session was last pulled (`liberarr status`).
    pub async fn last_pull(&self) -> Result<Option<jiff::Timestamp>> {
        let last: Option<String> = self
            .run(|c| {
                c.query_row("SELECT MAX(pulled_at) FROM device_sessions", [], |r| {
                    r.get(0)
                })
            })
            .await?;
        last.map(|s| s.parse().map_err(storage)).transpose()
    }
}
