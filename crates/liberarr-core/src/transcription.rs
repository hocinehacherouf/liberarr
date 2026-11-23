//! Transcription context: transcript model, chunk merging and the transcribe use case.
use crate::recordings::RecordingRepo;
use crate::{Error, Result};
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// What a provider returns for one chunk. `segments` is empty when it gives no timestamps.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkResult {
    pub language: Option<String>,
    pub text: String,
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub engine: String,
    pub language: Option<String>,
    pub text: String,
    pub segments: Vec<Segment>,
    /// False when the provider gave no segments: the text is shown without player sync.
    pub has_timestamps: bool,
}

impl Transcript {
    /// False for a silent recording: the summary and the note are skipped, the transcript is kept.
    pub fn has_speech(&self) -> bool {
        !self.text.trim().is_empty()
    }
}

/// One correction. `index` addresses a unit (see `units`); `original` is the engine's text of that unit
/// when the edit was made; a blank `text` removes the unit.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentEdit {
    pub index: usize,
    pub original: String,
    pub text: String,
}

impl SegmentEdit {
    pub fn removes(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn reverts(&self) -> bool {
        self.text.trim() == self.original.trim()
    }
}

/// One change sent by a client: the unit it changes, the text it displayed there (the corrected text, "" for a
/// removed segment) and the new text.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentChange {
    pub index: usize,
    pub current: String,
    pub text: String,
}

/// The segment texts, or `[text]` (one unit, index 0) when `!t.has_timestamps`: such a transcript's segments
/// may be partial, so the whole text is the unit.
pub fn units(t: &Transcript) -> Vec<&str> {
    if t.has_timestamps {
        t.segments.iter().map(|s| s.text.as_str()).collect()
    } else {
        vec![t.text.as_str()]
    }
}

/// The edit in force for each unit: the last one of its index whose `original` is still the raw unit.
fn in_force<'a>(raw: &Transcript, edits: &'a [SegmentEdit]) -> Vec<Option<&'a SegmentEdit>> {
    let units = units(raw);
    let mut slots = vec![None; units.len()];
    for e in edits {
        if units.get(e.index).is_some_and(|u| *u == e.original) {
            slots[e.index] = Some(e);
        }
    }
    slots
}

/// The corrected text of each unit, by raw index: the stored edit's text, "" when that edit removes the unit, else the
/// raw unit. An edit whose `original` differs from the raw unit is ignored, as in `apply_edits`.
pub fn unit_texts<'a>(raw: &'a Transcript, edits: &'a [SegmentEdit]) -> Vec<&'a str> {
    units(raw)
        .into_iter()
        .zip(in_force(raw, edits))
        .map(|(u, e)| match e {
            Some(e) if e.removes() => "",
            Some(e) => e.text.as_str(),
            None => u,
        })
        .collect()
}

/// Turns a batch into edits to store, each `original` taken from `raw`: `NotFound("segment")` for an index >=
/// units(raw).len(); `Conflict("segment")` when any `current` differs (exact) from unit_texts(raw, edits)[index].
pub fn prepare_edits(
    raw: &Transcript,
    edits: &[SegmentEdit],
    changes: &[SegmentChange],
) -> Result<Vec<SegmentEdit>> {
    let units = units(raw);
    let shown = unit_texts(raw, edits);
    changes
        .iter()
        .map(|c| {
            let original = units
                .get(c.index)
                .ok_or_else(|| Error::NotFound("segment".into()))?;
            if shown[c.index] != c.current {
                return Err(Error::Conflict("segment".into()));
            }
            Ok(SegmentEdit {
                index: c.index,
                original: (*original).into(),
                text: c.text.clone(),
            })
        })
        .collect()
}

/// What every reader gets. No edits: `t` unchanged. With timestamps: an edit replaces its segment's text, a removing
/// edit drops the segment, and `text` = the trimmed non-blank segment texts joined by " ". Without timestamps: an edit
/// at 0 replaces `text` and `segments` becomes empty. An edit whose `original` differs from the raw unit at its index is
/// ignored (a read racing a new transcript); out-of-range indexes are ignored; the last edit of an index wins.
pub fn apply_edits(mut t: Transcript, edits: &[SegmentEdit]) -> Transcript {
    let slots = in_force(&t, edits);
    if slots.iter().all(Option::is_none) {
        return t;
    }
    if !t.has_timestamps {
        // One unit: the only slot holds an edit.
        t.text = slots[0]
            .filter(|e| !e.removes())
            .map(|e| e.text.clone())
            .unwrap_or_default();
        t.segments.clear();
        return t;
    }
    t.segments = t
        .segments
        .into_iter()
        .zip(slots)
        .filter_map(|(s, e)| match e {
            Some(e) if e.removes() => None,
            Some(e) => Some(Segment {
                text: e.text.clone(),
                ..s
            }),
            None => Some(s),
        })
        .collect();
    t.text = t
        .segments
        .iter()
        .map(|s| s.text.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    t
}

/// `mm:ss`, minutes not wrapped at 60.
pub fn timestamp(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Joins per-chunk results; every timestamp is shifted by its chunk's offset in the full recording.
pub fn merge_chunks(results: Vec<(ChunkResult, f64)>, engine: &str) -> Transcript {
    let language = results.iter().find_map(|(r, _)| r.language.clone());
    let text = results
        .iter()
        .map(|(r, _)| r.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // Timestamps only when every chunk with text has segments, else the segment view drops text.
    let all_timed = results
        .iter()
        .all(|(r, _)| !r.segments.is_empty() || r.text.trim().is_empty());
    let segments: Vec<Segment> = results
        .into_iter()
        .flat_map(|(r, offset)| {
            r.segments.into_iter().map(move |s| Segment {
                start: s.start + offset,
                end: s.end + offset,
                text: s.text,
            })
        })
        .collect();
    Transcript {
        engine: engine.into(),
        language,
        text,
        has_timestamps: all_timed && !segments.is_empty(),
        segments,
    }
}

pub trait Transcriber {
    /// No forced language: recordings mix French and English.
    async fn transcribe(&self, audio: &[u8], filename: &str) -> Result<ChunkResult>;
    fn engine(&self) -> &str;
}

/// Splits audio into `(chunk bytes, offset in seconds)` of at most `max_bytes` each.
/// Refuses what it cannot split under the limit.
pub trait AudioSplitter {
    fn split(&self, path: &Path, max_bytes: usize) -> Result<Vec<(Vec<u8>, f64)>>;
}

pub trait TranscriptRepo {
    /// Drops the owner's edits when the new segments or text differ from the stored ones.
    async fn save(&self, recording_id: i64, transcript: &Transcript) -> Result<()>;
    /// With the owner's edits applied (`apply_edits`).
    async fn get(&self, recording_id: i64) -> Result<Option<Transcript>>;
}

pub trait TranscriptEditRepo {
    /// The engine's transcript, untouched, and the edits by ascending index.
    async fn get_with_edits(
        &self,
        recording_id: i64,
    ) -> Result<Option<(Transcript, Vec<SegmentEdit>)>>;
    /// One transaction: `NotFound("transcript")` without a transcript, then `prepare_edits` against the stored raw
    /// transcript and edits (any failure writes nothing), then each resulting edit is upserted, or deleted when it `reverts`.
    async fn set_edits(&self, recording_id: i64, changes: &[SegmentChange]) -> Result<()>;
}

pub struct Transcribe<'a, R, P, T, S> {
    pub recordings: &'a R,
    pub splitter: &'a P,
    pub transcriber: &'a T,
    pub transcripts: &'a S,
    /// Largest upload the transcription provider accepts.
    pub max_chunk_bytes: usize,
}

impl<R: RecordingRepo, P: AudioSplitter, T: Transcriber, S: TranscriptRepo>
    Transcribe<'_, R, P, T, S>
{
    pub async fn run(&self, recording_id: i64) -> Result<()> {
        let rec = self
            .recordings
            .get(recording_id)
            .await?
            .ok_or_else(|| Error::NotFound("recording".into()))?;
        let ext = rec
            .audio_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("ogg");
        let mut results = Vec::new();
        for (i, (bytes, offset)) in self
            .splitter
            .split(&rec.audio_path, self.max_chunk_bytes)?
            .into_iter()
            .enumerate()
        {
            results.push((
                self.transcriber
                    .transcribe(&bytes, &format!("chunk-{i}.{ext}"))
                    .await?,
                offset,
            ));
        }
        self.transcripts
            .save(
                recording_id,
                &merge_chunks(results, self.transcriber.engine()),
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, text: &str) -> Segment {
        Segment {
            start,
            end: start + 1.0,
            text: text.into(),
        }
    }

    #[test]
    fn merge_offsets_timestamps() {
        let first = ChunkResult {
            language: Some("fr".into()),
            text: "a".into(),
            segments: vec![seg(1.0, "a")],
        };
        let second = ChunkResult {
            language: None,
            text: "b".into(),
            segments: vec![seg(1.0, "b")],
        };
        let t = merge_chunks(vec![(first, 0.0), (second, 1200.0)], "whisper");
        assert_eq!(t.segments[0].start, 1.0);
        assert_eq!(t.segments[1].start, 1201.0);
        assert_eq!(t.segments[1].end, 1202.0);
        assert_eq!(t.text, "a b");
        assert_eq!(t.language.as_deref(), Some("fr"));
        assert!(t.has_timestamps);
    }

    #[test]
    fn transcript_without_timestamps_is_kept() {
        let c = ChunkResult {
            language: None,
            text: " bonjour tout le monde ".into(),
            segments: vec![],
        };
        let t = merge_chunks(vec![(c, 0.0)], "whisper");
        assert!(!t.has_timestamps);
        assert!(t.segments.is_empty());
        assert_eq!(t.text, "bonjour tout le monde");
    }

    #[test]
    fn one_chunk_without_segments_disables_timestamps() {
        let with = ChunkResult {
            language: None,
            text: "a".into(),
            segments: vec![seg(1.0, "a")],
        };
        let without = ChunkResult {
            language: None,
            text: "b".into(),
            segments: vec![],
        };
        // Otherwise the UI and the note show segments only and "b" disappears.
        let t = merge_chunks(vec![(with, 0.0), (without, 1200.0)], "whisper");
        assert!(!t.has_timestamps);
        assert_eq!(t.text, "a b");
    }

    fn timed(texts: &[&str]) -> Transcript {
        Transcript {
            engine: "e".into(),
            language: None,
            text: texts.iter().map(|t| t.trim()).collect::<Vec<_>>().join(" "),
            segments: texts
                .iter()
                .enumerate()
                .map(|(i, t)| seg(i as f64 * 2.0, t))
                .collect(),
            has_timestamps: true,
        }
    }

    fn edit(index: usize, original: &str, text: &str) -> SegmentEdit {
        SegmentEdit {
            index,
            original: original.into(),
            text: text.into(),
        }
    }

    #[test]
    fn apply_edits_replaces_removes_and_rebuilds_the_text() {
        let t = timed(&[" Bonjour Jon", " ça va", " merci"]);
        assert_eq!(apply_edits(t.clone(), &[]), t);
        // An edit made on another engine text (a read racing a new transcript) is ignored.
        assert_eq!(apply_edits(t.clone(), &[edit(0, " Bonjour Jean", "X")]), t);
        let out = apply_edits(
            t.clone(),
            &[
                edit(0, " Bonjour Jon", "Bonjour John"),
                edit(1, " ça va", "  "),
            ],
        );
        let segs: Vec<_> = out
            .segments
            .iter()
            .map(|s| (s.start, s.text.as_str()))
            .collect();
        assert_eq!(segs, [(0.0, "Bonjour John"), (4.0, " merci")]);
        assert_eq!(out.text, "Bonjour John merci");
        assert!(out.has_timestamps);
        // Everything removed: no speech, so the summary and the note skip it.
        let all: Vec<_> = t
            .segments
            .iter()
            .enumerate()
            .map(|(i, s)| edit(i, &s.text, ""))
            .collect();
        assert!(!apply_edits(t, &all).has_speech());
    }

    #[test]
    fn transcript_without_timestamps_is_one_unit() {
        let t = Transcript {
            engine: "e".into(),
            language: None,
            text: "bonjour jon".into(),
            segments: vec![seg(0.0, "bonjour")], // partial: one chunk had no segments
            has_timestamps: false,
        };
        assert_eq!(units(&t), ["bonjour jon"]);
        let change = SegmentChange {
            index: 1,
            current: "x".into(),
            text: "y".into(),
        };
        assert!(matches!(
            prepare_edits(&t, &[], &[change]),
            Err(Error::NotFound(_))
        ));
        let out = apply_edits(t.clone(), &[edit(0, "bonjour jon", "bonjour John")]);
        assert_eq!(out.text, "bonjour John");
        assert!(out.segments.is_empty() && !out.has_timestamps);
        // A removal sent as whitespace reads "" like `unit_texts`, or the page would get 409 on every later save.
        assert_eq!(apply_edits(t, &[edit(0, "bonjour jon", "  ")]).text, "");
    }
}
