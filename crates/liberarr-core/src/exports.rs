//! Exports context: one-way Obsidian note rendering and marker-bounded upsert.
use crate::lang::Language;
use crate::recordings::{Recording, RecordingRepo};
use crate::summaries::{MeetingSummary, SummaryRepo};
use crate::transcription::{Transcript, TranscriptRepo, timestamp};
use crate::{Error, Result};
use jiff::tz::TimeZone;
use std::path::{Path, PathBuf};

/// Liberarr owns only the text between these markers; the owner's edits around them are never touched.
pub const START: &str = "%% liberarr:start %%";
pub const END: &str = "%% liberarr:end %%";

fn local(r: &Recording, tz: &TimeZone) -> jiff::Zoned {
    r.created_at.to_zoned(tz.clone())
}

/// `YYYY-MM-DD HHmm - <title>.md` in local time, with path-unsafe and control characters removed.
pub fn note_filename(r: &Recording, title: &str, tz: &TimeZone) -> String {
    let stamp = local(r, tz).strftime("%Y-%m-%d %H%M");
    let safe: String = title
        .chars()
        .filter(|c| !"/\\:*?\"<>|".contains(*c) && (c.is_whitespace() || !c.is_control()))
        .collect();
    // One line (newlines and tabs become single spaces), capped at 150 bytes on a char boundary:
    // file systems limit a name to 255 bytes, and CJK/emoji titles are 3-4 bytes per character.
    let mut bytes = 0;
    let safe: String = safe
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take_while(|c| {
            bytes += c.len_utf8();
            bytes <= 150
        })
        .collect();
    let safe = safe.trim();
    if safe.is_empty() {
        format!("{stamp} - recording {}.md", r.id)
    } else {
        format!("{stamp} - {safe}.md")
    }
}

pub fn frontmatter(r: &Recording, tz: &TimeZone) -> String {
    format!(
        "---\ndate: {}\ntype: meeting\nsource: {}\nliberarr_id: {}\ntags: [liberarr]\n---",
        local(r, tz).strftime("%Y-%m-%d"),
        r.source.as_str(),
        r.id
    )
}

/// Without a summary only the transcript section is written, so a note exists before (or without) an AI run.
pub fn render_block(t: &Transcript, s: Option<&MeetingSummary>, lang: Language) -> String {
    let l = lang.note_labels();
    let transcript: Vec<String> = if t.has_timestamps {
        t.segments
            .iter()
            .map(|g| format!("[{}] {}", timestamp(g.start), g.text.trim()))
            .collect()
    } else {
        vec![t.text.clone()]
    };
    let mut lines = Vec::new();
    if let Some(s) = s {
        let actions: Vec<String> = s
            .actions
            .iter()
            .map(|a| {
                let parts: Vec<&str> = [
                    Some(a.action.as_str()),
                    a.owner.as_deref(),
                    a.due.as_deref(),
                ]
                .into_iter()
                .flatten()
                .collect();
                format!(
                    "- [{}] {}",
                    if a.done { "x" } else { " " },
                    parts.join(" — ")
                )
            })
            .collect();
        lines.extend([
            format!("## {}", l.summary),
            s.summary_md.trim().into(),
            String::new(),
            format!("## {}", l.mindmap),
            "```mermaid".into(),
            s.mindmap.trim().into(),
            "```".into(),
            String::new(),
            format!("## {}", l.actions),
        ]);
        lines.extend(if actions.is_empty() {
            vec![format!("_{}_", l.no_actions)]
        } else {
            actions
        });
        lines.push(String::new());
    }
    lines.push(format!("## {}", l.transcript));
    lines.extend(transcript);
    // LLM and transcript text must not close or reopen the block: strip both markers, repeating
    // because a removal can join the surrounding text into a new marker.
    let mut body = lines.join("\n");
    while body.contains(START) || body.contains(END) {
        body = body.replace(START, "").replace(END, "");
    }
    format!("{START}\n{body}\n{END}")
}

/// Replaces the block between the last START that has an END after it; text outside is kept byte
/// for byte. With no such pair (new note, or the owner deleted an END) the block is appended, so an
/// orphan START and the text after it survive.
pub fn upsert_block(existing: Option<&str>, block: &str, front: &str) -> String {
    let Some(existing) = existing else {
        return format!("{front}\n{block}\n");
    };
    // From the end: an orphan START after the real block must not hide it.
    if let Some((start, end)) = existing
        .rmatch_indices(START)
        .find_map(|(start, _)| Some((start, existing[start..].find(END)?)))
    {
        return format!(
            "{}{}{}",
            &existing[..start],
            block,
            &existing[start + end + END.len()..]
        );
    }
    format!("{}\n\n{}\n", existing.trim_end_matches('\n'), block)
}

pub trait NoteStore {
    fn read(&self, path: &Path) -> Result<Option<String>>;
    /// Writes to a temp file and renames, so a crash never leaves a half-written note.
    fn write_atomic(&self, path: &Path, text: &str) -> Result<()>;
}

pub struct ExportToObsidian<'a, R, T, S, N> {
    pub recordings: &'a R,
    pub transcripts: &'a T,
    pub summaries: &'a S,
    pub notes: &'a N,
    pub vault: PathBuf,
    pub tz: TimeZone,
    pub lang: Language,
}

impl<R: RecordingRepo, T: TranscriptRepo, S: SummaryRepo, N: NoteStore>
    ExportToObsidian<'_, R, T, S, N>
{
    /// Writes the note and returns its path. The file name is fixed on the first export (` (<id>)`
    /// is added when the name is taken), so later exports rewrite the same note. Only the name is
    /// stored: the vault can move without stranding old recordings. `None` (no note) when the
    /// transcript has no speech.
    pub async fn run(&self, recording_id: i64) -> Result<Option<PathBuf>> {
        let rec = self
            .recordings
            .get(recording_id)
            .await?
            .ok_or_else(|| Error::NotFound("recording".into()))?;
        let t = self
            .transcripts
            .get(recording_id)
            .await?
            .ok_or_else(|| Error::NotFound("transcript".into()))?;
        if !t.has_speech() {
            return Ok(None);
        }
        let s = self.summaries.get(recording_id).await?;
        // `join` keeps a legacy absolute value working.
        let path = match rec.export_path.clone() {
            Some(name) => self.vault.join(name),
            None => {
                // shortcut: check-then-write is not atomic, so two exports of the same name at the
                // same time (serve, tui and process share the DB) can both see it free and the second
                // block replaces the first. Upgrade to an atomic create (`create_new`) in `NoteStore`.
                let mut name = note_filename(
                    &rec,
                    s.as_ref()
                        .map(|s| s.title.as_str())
                        .or(rec.title.as_deref())
                        .unwrap_or(""),
                    &self.tz,
                );
                if self.notes.read(&self.vault.join(&name))?.is_some() {
                    name = format!(
                        "{} ({}).md",
                        name.strip_suffix(".md").unwrap_or(&name),
                        rec.id
                    );
                }
                // Fixed before writing: a crash in between retries the same name, never a second note.
                self.recordings
                    .set_export_path(rec.id, Path::new(&name))
                    .await?;
                self.vault.join(name)
            }
        };
        let text = upsert_block(
            self.notes.read(&path)?.as_deref(),
            &render_block(&t, s.as_ref(), self.lang),
            &frontmatter(&rec, &self.tz),
        );
        self.notes.write_atomic(&path, &text)?;
        Ok(Some(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{MemNotes, MemRecordings, MemSummaries, MemTranscripts, block_on};
    use crate::recordings::{NewRecording, RecordingRepo, Source};
    use crate::summaries::{MeetingSummary, SummaryRepo};
    use crate::transcription::{Segment, Transcript, TranscriptRepo};
    use jiff::tz::TimeZone;

    fn transcript() -> Transcript {
        Transcript {
            engine: "e".into(),
            language: None,
            text: "salut".into(),
            segments: vec![Segment {
                start: 65.0,
                end: 70.0,
                text: "salut".into(),
            }],
            has_timestamps: true,
        }
    }

    fn summary() -> MeetingSummary {
        MeetingSummary {
            title: "Point hebdo".into(),
            summary_md: "## Contexte".into(),
            mindmap: "mindmap\n  root".into(),
            actions: vec![],
        }
    }

    fn new_rec(created_at: &str) -> NewRecording {
        NewRecording {
            title: None,
            source: Source::File,
            source_ref: created_at.into(),
            audio_path: "/a.ogg".into(),
            duration_s: None,
            created_at: created_at.parse().unwrap(),
        }
    }

    #[test]
    fn upsert_preserves_outside_text() {
        let first = upsert_block(
            None,
            "%% liberarr:start %%\nv1\n%% liberarr:end %%",
            "---\nfm\n---",
        );
        let edited = format!("{}\nmy own notes é\n", first.replace("v1", "v1 kept"));
        let owner_text = "\nmy own notes é\n";
        let out = upsert_block(
            Some(&edited),
            "%% liberarr:start %%\nv2\n%% liberarr:end %%",
            "ignored",
        );
        assert!(out.ends_with(owner_text));
        assert_eq!(out, edited.replace("v1 kept", "v2"));
    }

    #[test]
    fn markers_in_generated_text_do_not_split_the_block() {
        let mut s = summary();
        s.summary_md = format!("a {END} b {START} c %% liberarr:st{END}art %%");
        let block = render_block(&transcript(), Some(&s), Language::En);
        assert_eq!(
            (block.matches(START).count(), block.matches(END).count()),
            (1, 1)
        );
        let once = upsert_block(None, &block, "fm");
        let twice = upsert_block(Some(&once), &block, "fm");
        assert_eq!(once, twice);
    }

    #[test]
    fn upsert_orphan_start_keeps_owner_text() {
        let existing = "intro\n%% liberarr:start %%\nowner wrote this, deleted the end marker\n";
        let block = "%% liberarr:start %%\nv1\n%% liberarr:end %%";
        let once = upsert_block(Some(existing), block, "");
        let twice = upsert_block(Some(&once), &block.replace("v1", "v2"), "");
        assert!(twice.starts_with(existing.trim_end_matches('\n')));
        assert!(twice.contains("owner wrote this, deleted the end marker"));
        assert!(twice.contains("v2") && !twice.contains("v1"));
        assert_eq!(twice.matches(END).count(), 1);
    }

    #[test]
    fn upsert_skips_trailing_orphan_start() {
        let existing = "top\n%% liberarr:start %%\nold\n%% liberarr:end %%\nmid\n%% liberarr:start %%\nowner text\n";
        let out = upsert_block(
            Some(existing),
            "%% liberarr:start %%\nnew\n%% liberarr:end %%",
            "",
        );
        assert_eq!(
            out,
            "top\n%% liberarr:start %%\nnew\n%% liberarr:end %%\nmid\n%% liberarr:start %%\nowner text\n"
        );
    }

    #[test]
    fn note_filename_uses_local_time() {
        let rec = Recording {
            id: 1,
            title: None,
            source: Source::File,
            source_ref: "r".into(),
            audio_path: "/a.ogg".into(),
            duration_s: None,
            created_at: "2026-06-02T14:05:00Z".parse().unwrap(),
            export_path: None,
        };
        let paris = TimeZone::get("Europe/Paris").unwrap();
        assert_eq!(
            note_filename(&rec, "Point/hebdo: \"v2\"?", &paris),
            "2026-06-02 1605 - Pointhebdo v2.md"
        );
        // Winter (CET, UTC+1): the brief's 14:05 UTC -> 1505.
        let winter = Recording {
            created_at: "2026-01-15T14:05:00Z".parse().unwrap(),
            ..rec
        };
        assert_eq!(
            note_filename(&winter, "Point", &paris),
            "2026-01-15 1505 - Point.md"
        );
        // LLM titles are untrusted: one line, bounded in bytes (file systems cap names at 255 bytes).
        let long = format!("Ligne un\nligne\tdeux\r\x07 {}", "é".repeat(300));
        let name = note_filename(&winter, &long, &paris);
        let title = &name["2026-01-15 1505 - ".len()..name.len() - ".md".len()];
        assert!(title.starts_with("Ligne un ligne deux éé"), "{title}");
        assert!(!name.chars().any(char::is_control));
        assert!(name.len() <= 255, "{}", name.len());
        for wide in ["😀".repeat(300), "会议纪要".repeat(100)] {
            let name = note_filename(&winter, &wide, &paris);
            // Room for the ` (<id>)` suffix added on a name clash.
            assert!(
                name.len() + " (9999999999).md".len() <= 255,
                "{}",
                name.len()
            );
        }
    }

    #[test]
    fn export_same_name_gets_distinct_path() {
        let recs = MemRecordings::default();
        let (transcripts, summaries, notes) = (
            MemTranscripts::default(),
            MemSummaries::default(),
            MemNotes::default(),
        );
        for _ in 0..2 {
            let id = block_on(recs.add(new_rec("2026-06-02T14:05:00Z"))).unwrap();
            block_on(transcripts.save(id, &transcript())).unwrap();
            block_on(summaries.save(id, &summary())).unwrap();
        }
        let uc = ExportToObsidian {
            recordings: &recs,
            transcripts: &transcripts,
            summaries: &summaries,
            notes: &notes,
            vault: "/vault".into(),
            tz: TimeZone::UTC,
            lang: Language::Fr,
        };
        let a = block_on(uc.run(1)).unwrap().unwrap();
        let b = block_on(uc.run(2)).unwrap().unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with(" (2).md"));
        // The path is fixed on the first export: re-running rewrites the same note.
        assert_eq!(block_on(uc.run(1)).unwrap(), Some(a.clone()));
        assert_eq!(notes.0.borrow().len(), 2);
        assert_eq!(
            recs.0.borrow()[0].export_path.as_deref(),
            a.file_name().map(Path::new)
        );
    }

    fn export_fixture() -> (MemRecordings, MemTranscripts, MemSummaries, MemNotes) {
        (
            MemRecordings::default(),
            MemTranscripts::default(),
            MemSummaries::default(),
            MemNotes::default(),
        )
    }

    #[test]
    fn export_without_summary_writes_transcript_only() {
        let (recs, transcripts, summaries, notes) = export_fixture();
        let id = block_on(recs.add(NewRecording {
            title: Some("Point équipe".into()),
            ..new_rec("2026-06-02T14:05:00Z")
        }))
        .unwrap();
        block_on(transcripts.save(id, &transcript())).unwrap();
        let uc = ExportToObsidian {
            recordings: &recs,
            transcripts: &transcripts,
            summaries: &summaries,
            notes: &notes,
            vault: "/vault".into(),
            tz: TimeZone::UTC,
            lang: Language::Fr,
        };
        let path = block_on(uc.run(id)).unwrap().unwrap();
        assert!(path.to_string_lossy().ends_with("- Point équipe.md"));
        let note = notes.read(&path).unwrap().unwrap();
        let l = Language::Fr.note_labels();
        assert!(note.contains(START));
        assert!(note.contains(&format!("## {}", l.transcript)));
        assert!(note.contains("[01:05] salut"));
        assert!(!note.contains(&format!("## {}", l.summary)));
    }

    #[test]
    fn export_without_summary_then_with_summary() {
        let (recs, transcripts, summaries, notes) = export_fixture();
        let id = block_on(recs.add(new_rec("2026-06-02T14:05:00Z"))).unwrap();
        block_on(transcripts.save(id, &transcript())).unwrap();
        let uc = ExportToObsidian {
            recordings: &recs,
            transcripts: &transcripts,
            summaries: &summaries,
            notes: &notes,
            vault: "/vault".into(),
            tz: TimeZone::UTC,
            lang: Language::Fr,
        };
        let first = block_on(uc.run(id)).unwrap().unwrap();
        let before = notes.read(&first).unwrap().unwrap();
        block_on(summaries.save(
            id,
            &MeetingSummary {
                title: "Autre titre".into(),
                ..summary()
            },
        ))
        .unwrap();
        let second = block_on(uc.run(id)).unwrap().unwrap();
        assert_eq!(first, second);
        let after = notes.read(&second).unwrap().unwrap();
        assert!(after.contains(&format!("## {}", Language::Fr.note_labels().summary)));
        assert!(after.contains("## Contexte"));
        let outside = |t: &str| {
            let (a, b) = (t.find(START).unwrap(), t.find(END).unwrap() + END.len());
            format!("{}{}", &t[..a], &t[b..])
        };
        assert_eq!(outside(&before), outside(&after));
    }

    #[test]
    fn export_without_speech_is_a_no_op() {
        let recs = MemRecordings::default();
        let (transcripts, summaries, notes) = (
            MemTranscripts::default(),
            MemSummaries::default(),
            MemNotes::default(),
        );
        let id = block_on(recs.add(new_rec("2026-06-02T14:05:00Z"))).unwrap();
        let silent = Transcript {
            text: " ".into(),
            segments: vec![],
            ..transcript()
        };
        block_on(transcripts.save(id, &silent)).unwrap();
        let uc = ExportToObsidian {
            recordings: &recs,
            transcripts: &transcripts,
            summaries: &summaries,
            notes: &notes,
            vault: "/vault".into(),
            tz: TimeZone::UTC,
            lang: Language::Fr,
        };
        // Silence beats a missing summary: nothing is written.
        assert_eq!(block_on(uc.run(id)).unwrap(), None);
        assert!(notes.0.borrow().is_empty());
        assert_eq!(recs.0.borrow()[0].export_path, None);
    }
}
