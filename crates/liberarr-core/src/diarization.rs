//! Diarization context: who spoke when, matched onto the transcript segments. Local only: the
//! audio and the voice embeddings never leave the machine (the adapter runs the models).
use crate::recordings::RecordingRepo;
use crate::transcription::TranscriptRepo;
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// A segment must overlap a turn by at least this share of its length to get its speaker.
const MIN_OVERLAP: f64 = 0.3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeakerTurn {
    pub start: f64,
    pub end: f64,
    /// 0-based; shown as "Speaker {speaker + 1}".
    pub speaker: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Diarization {
    pub turns: Vec<SpeakerTurn>,
    pub engine: String,
}

pub trait Diarizer {
    /// `speakers` is the count the owner set for the recording; `None` lets the engine decide.
    async fn diarize(&self, audio_path: &Path, speakers: Option<u32>) -> Result<Vec<SpeakerTurn>>;
    fn engine(&self) -> &str;
}

pub trait DiarizationRepo {
    async fn get(&self, recording_id: i64) -> Result<Option<Diarization>>;
    async fn save(&self, recording_id: i64, d: &Diarization) -> Result<()>;
    /// Kept across reruns.
    async fn requested_speakers(&self, recording_id: i64) -> Result<Option<u32>>;
    async fn set_requested_speakers(&self, recording_id: i64, n: Option<u32>) -> Result<()>;
    async fn names(&self, recording_id: i64) -> Result<BTreeMap<u32, String>>;
    /// `None` or a blank name resets the speaker to its default label.
    async fn set_name(&self, recording_id: i64, speaker: u32, name: Option<&str>) -> Result<()>;
}

/// The speaker with the largest total overlap with `[start, end]`; `None` when no speaker covers
/// at least 30 % of the segment (or the segment has no length). Ties go to the lower number.
pub fn speaker_for(start: f64, end: f64, turns: &[SpeakerTurn]) -> Option<u32> {
    let len = end - start;
    if len <= 0.0 {
        return None;
    }
    let mut overlaps = BTreeMap::<u32, f64>::new();
    for t in turns {
        let o = end.min(t.end) - start.max(t.start);
        if o > 0.0 {
            *overlaps.entry(t.speaker).or_default() += o;
        }
    }
    // BTreeMap order + strict `>`: the first of equal maxima is the lowest speaker.
    let mut best: Option<(u32, f64)> = None;
    for (speaker, o) in overlaps {
        if best.is_none_or(|(_, b)| o > b) {
            best = Some((speaker, o));
        }
    }
    best.filter(|(_, o)| *o >= MIN_OVERLAP * len)
        .map(|(s, _)| s)
}

/// The owner's name for the speaker, else "{default} {n + 1}" (`default` is the translated word).
pub fn speaker_label(speaker: u32, names: &BTreeMap<u32, String>, default: &str) -> String {
    names
        .get(&speaker)
        .cloned()
        .unwrap_or_else(|| format!("{default} {}", speaker + 1))
}

pub struct Diarize<'a, R, T, D, S> {
    pub recordings: &'a R,
    pub transcripts: &'a T,
    pub diarizer: &'a D,
    pub store: &'a S,
}

impl<R: RecordingRepo, T: TranscriptRepo, D: Diarizer, S: DiarizationRepo> Diarize<'_, R, T, D, S> {
    /// Skips (Ok, nothing saved) when no segment could be labelled: no transcript, no speech or
    /// no timestamps.
    pub async fn run(&self, recording_id: i64) -> Result<()> {
        let labellable = self
            .transcripts
            .get(recording_id)
            .await?
            .is_some_and(|t| t.has_speech() && t.has_timestamps);
        if !labellable {
            return Ok(());
        }
        let rec = self
            .recordings
            .get(recording_id)
            .await?
            .ok_or_else(|| Error::NotFound("recording".into()))?;
        let speakers = self.store.requested_speakers(recording_id).await?;
        let turns = self.diarizer.diarize(&rec.audio_path, speakers).await?;
        let d = Diarization {
            turns,
            engine: self.diarizer.engine().into(),
        };
        self.store.save(recording_id, &d).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeDiarizer, MemDiarizations, MemRecordings, MemTranscripts, block_on};
    use crate::recordings::{NewRecording, Source};
    use crate::transcription::{Segment, Transcript};

    fn turn(start: f64, end: f64, speaker: u32) -> SpeakerTurn {
        SpeakerTurn {
            start,
            end,
            speaker,
        }
    }

    #[test]
    fn speaker_for_picks_the_largest_overlap() {
        // Covers the segment.
        assert_eq!(speaker_for(1.0, 3.0, &[turn(0.0, 10.0, 2)]), Some(2));
        // Straddle 60/40.
        let turns = [turn(0.0, 6.0, 1), turn(6.0, 10.0, 0)];
        assert_eq!(speaker_for(0.0, 10.0, &turns), Some(1));
        // 20 % is not enough; neither is nothing or an empty segment.
        assert_eq!(speaker_for(0.0, 10.0, &[turn(0.0, 2.0, 0)]), None);
        assert_eq!(speaker_for(0.0, 10.0, &[]), None);
        assert_eq!(speaker_for(5.0, 5.0, &[turn(0.0, 10.0, 0)]), None);
        // Same answer in any order; a tie goes to the lower speaker.
        let unsorted = [turn(6.0, 10.0, 0), turn(0.0, 6.0, 1)];
        assert_eq!(speaker_for(0.0, 10.0, &unsorted), Some(1));
        let tie = [turn(5.0, 10.0, 3), turn(0.0, 5.0, 1)];
        assert_eq!(speaker_for(0.0, 10.0, &tie), Some(1));
    }

    fn setup(t: Option<Transcript>) -> (MemRecordings, MemTranscripts, MemDiarizations) {
        let recs = MemRecordings::default();
        let id = block_on(recs.add(NewRecording {
            title: None,
            source: Source::File,
            source_ref: "r".into(),
            audio_path: "/audio/r.ogg".into(),
            duration_s: Some(5.0),
            created_at: jiff::Timestamp::UNIX_EPOCH,
        }))
        .unwrap();
        let transcripts = MemTranscripts::default();
        if let Some(t) = t {
            block_on(crate::transcription::TranscriptRepo::save(
                &transcripts,
                id,
                &t,
            ))
            .unwrap();
        }
        (recs, transcripts, MemDiarizations::default())
    }

    fn transcript(text: &str, has_timestamps: bool) -> Transcript {
        Transcript {
            engine: "e".into(),
            language: None,
            text: text.into(),
            segments: vec![Segment {
                start: 0.0,
                end: 2.0,
                text: text.into(),
            }],
            has_timestamps,
        }
    }

    #[test]
    fn diarize_skips_what_it_cannot_label() {
        for t in [
            None,
            Some(transcript("bonjour", false)),
            Some(transcript("  ", true)),
        ] {
            let (recs, transcripts, store) = setup(t);
            let diarizer = FakeDiarizer::default();
            let uc = Diarize {
                recordings: &recs,
                transcripts: &transcripts,
                diarizer: &diarizer,
                store: &store,
            };
            block_on(uc.run(1)).unwrap();
            assert_eq!(diarizer.calls.get(), 0);
            assert!(store.saved.borrow().is_empty());
        }
    }

    #[test]
    fn diarize_passes_the_requested_count_and_saves_the_turns() {
        let (recs, transcripts, store) = setup(Some(transcript("bonjour", true)));
        block_on(store.set_requested_speakers(1, Some(2))).unwrap();
        let diarizer = FakeDiarizer::default();
        let uc = Diarize {
            recordings: &recs,
            transcripts: &transcripts,
            diarizer: &diarizer,
            store: &store,
        };
        block_on(uc.run(1)).unwrap();
        assert_eq!(diarizer.calls.get(), 1);
        assert_eq!(diarizer.last_count.get(), Some(2));
        let saved = block_on(store.get(1)).unwrap().unwrap();
        assert_eq!(saved.turns, [turn(0.0, 2.0, 0)]);
        assert_eq!(saved.engine, "fake");
    }
}
