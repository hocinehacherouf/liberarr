//! Local speaker diarization with sherpa-onnx (pyannote segmentation + TitaNet embeddings, onnxruntime on
//! the CPU). Everything runs in this process: the audio and the voice embeddings never leave the machine.
//! Models are downloaded once by [`ModelStore`].
use crate::audio_prep::decode_opus_16k;
use crate::models::{ModelStore, PYANNOTE_SEGMENTATION, TITANET_SMALL};
use liberarr_core::audio::ogg::opus_packets;
use liberarr_core::diarization::{Diarizer, SpeakerTurn};
use liberarr_core::{Error, Result};
use sherpa_onnx::{OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig};
use std::path::{Path, PathBuf};

const ENGINE: &str = "pyannote-3.0+titanet-small";
/// Distance above which two voices are different speakers. sherpa's default (0.5) split one voice into
/// 21 speakers with TitaNet; 0.85 gave the right count on labelled test files but split one voice in two
/// on the owner's recordings, so 0.9. Ignored when the owner set the number of speakers.
const CLUSTER_THRESHOLD: f32 = 0.9;
/// In automatic mode, a speaker who talks less than this in total is a clustering artefact, not a person.
const MIN_SPEAKER_TOTAL_S: f64 = 5.0;

pub struct LocalDiarizer {
    models: ModelStore,
}

impl LocalDiarizer {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models: ModelStore::new(models_dir),
        }
    }

    /// True when a model file is not yet on disk (the first run downloads about 46 MB).
    pub fn models_missing(&self) -> bool {
        !(self.models.is_present(&PYANNOTE_SEGMENTATION) && self.models.is_present(&TITANET_SMALL))
    }
}

fn failed(what: &str) -> Error {
    Error::Provider(format!("local diarization: {what}"))
}

/// sherpa's speaker ids to turns, numbered by first appearance so "Speaker 1" speaks first.
fn renumber(segments: &[(f32, f32, i32)]) -> Vec<SpeakerTurn> {
    let mut seen: Vec<i32> = Vec::new();
    segments
        .iter()
        .map(|&(start, end, id)| {
            let speaker = seen.iter().position(|&s| s == id).unwrap_or_else(|| {
                seen.push(id);
                seen.len() - 1
            });
            SpeakerTurn {
                start: f64::from(start),
                end: f64::from(end),
                speaker: u32::try_from(speaker).unwrap_or(u32::MAX),
            }
        })
        .collect()
}

/// Folds "micro speakers" (total talk time under `min_total_s`) into the nearest non-micro speaker in
/// time (previous turn wins a tie). When every speaker is micro, the one who talks most absorbs the rest.
/// Turn boundaries are untouched; speakers are renumbered by first appearance. `turns` is in time order.
fn merge_micro_speakers(mut turns: Vec<SpeakerTurn>, min_total_s: f64) -> Vec<SpeakerTurn> {
    let mut totals: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
    for t in &turns {
        *totals.entry(t.speaker).or_default() += t.end - t.start;
    }
    let mut keep: Vec<u32> = totals
        .iter()
        .filter(|&(_, &total)| total >= min_total_s)
        .map(|(&s, _)| s)
        .collect();
    if keep.is_empty() {
        // Ties on the total go to the lowest id, so the result does not depend on hash order.
        keep.extend(
            totals
                .iter()
                .max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(a.0)))
                .map(|(&s, _)| s),
        );
    }
    let original = turns.clone();
    for (i, turn) in turns.iter_mut().enumerate() {
        if keep.contains(&turn.speaker) {
            continue;
        }
        let kept = |t: &&SpeakerTurn| keep.contains(&t.speaker);
        let prev = original[..i].iter().rev().find(kept);
        let next = original[i + 1..].iter().find(kept);
        let gap_prev = prev.map(|p| (turn.start - p.end).max(0.0));
        let gap_next = next.map(|n| (n.start - turn.end).max(0.0));
        turn.speaker = match (prev, next, gap_prev, gap_next) {
            (Some(p), Some(n), Some(a), Some(b)) => if a <= b { p } else { n }.speaker,
            (Some(p), ..) => p.speaker,
            (None, Some(n), ..) => n.speaker,
            _ => turn.speaker,
        };
    }
    let mut seen: Vec<u32> = Vec::new();
    for turn in &mut turns {
        turn.speaker = match seen.iter().position(|&s| s == turn.speaker) {
            Some(p) => p,
            None => {
                seen.push(turn.speaker);
                seen.len() - 1
            }
        } as u32;
    }
    turns
}

impl Diarizer for LocalDiarizer {
    async fn diarize(&self, audio_path: &Path, speakers: Option<u32>) -> Result<Vec<SpeakerTurn>> {
        // Cheap check first: a bad file must not trigger the model download. No CPU check: onnxruntime
        // picks its kernels at run time, so this works where whisper.cpp does not. Off the async
        // threads: the file can be large.
        let path = audio_path.to_path_buf();
        let audio = tokio::task::spawn_blocking(move || {
            let audio = std::fs::read(path).map_err(|_| failed("cannot read the audio"))?;
            if opus_packets(&audio).is_err() {
                return Err(Error::Unsupported(
                    "local diarization reads Ogg Opus only".into(),
                ));
            }
            Ok(audio)
        })
        .await
        .map_err(|_| failed("worker stopped"))??;
        let segmentation = self.models.ensure(&PYANNOTE_SEGMENTATION).await?;
        let embedding = self.models.ensure(&TITANET_SMALL).await?;
        tokio::task::spawn_blocking(move || run(&audio, &segmentation, &embedding, speakers))
            .await
            .map_err(|_| failed("worker stopped"))?
    }

    fn engine(&self) -> &str {
        ENGINE
    }
}

/// Decode, then diarize. The models live for this call only, so Whisper's memory is free by then.
fn run(
    audio: &[u8],
    segmentation: &Path,
    embedding: &Path,
    speakers: Option<u32>,
) -> Result<Vec<SpeakerTurn>> {
    // Not normalised, unlike transcription: the embeddings should hear the real levels.
    let samples = decode_opus_16k(audio)?;

    let threads = std::thread::available_parallelism()
        .map_or(4, usize::from)
        .min(4);
    let threads = i32::try_from(threads).unwrap_or(4);
    let mut config = OfflineSpeakerDiarizationConfig::default();
    config.segmentation.pyannote.model = Some(path_str(segmentation)?);
    config.segmentation.num_threads = threads;
    config.embedding.model = Some(path_str(embedding)?);
    config.embedding.num_threads = threads;
    match speakers {
        Some(n) if n > 0 => config.clustering.num_clusters = i32::try_from(n).unwrap_or(i32::MAX),
        _ => config.clustering.threshold = CLUSTER_THRESHOLD,
    }
    let diarizer = OfflineSpeakerDiarization::create(&config)
        .ok_or_else(|| failed("cannot load the models"))?;
    let result = diarizer
        .process(&samples)
        .ok_or_else(|| failed("diarization failed"))?;
    let segments: Vec<(f32, f32, i32)> = result
        .sort_by_start_time()
        .iter()
        .map(|s| (s.start, s.end, s.speaker))
        .collect();
    let turns = renumber(&segments);
    // Merging needs the sherpa clustering free to choose the count; a count set by the owner is respected.
    Ok(match speakers {
        Some(n) if n > 0 => turns,
        _ => merge_micro_speakers(turns, MIN_SPEAKER_TOTAL_S),
    })
}

fn path_str(p: &Path) -> Result<String> {
    p.to_str()
        .map(str::to_string)
        .ok_or_else(|| failed("bad model path"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f32, end: f32, speaker: i32) -> (f32, f32, i32) {
        (start, end, speaker)
    }

    #[test]
    fn speakers_are_numbered_by_first_appearance() {
        let turns = renumber(&[seg(0.0, 1.0, 3), seg(1.0, 2.0, 0), seg(2.0, 3.0, 3)]);
        assert_eq!(
            turns.iter().map(|t| t.speaker).collect::<Vec<_>>(),
            [0, 1, 0]
        );
        assert_eq!((turns[1].start, turns[1].end), (1.0, 2.0));
    }

    fn turn(start: f64, end: f64, speaker: u32) -> SpeakerTurn {
        SpeakerTurn {
            start,
            end,
            speaker,
        }
    }

    fn speakers(turns: &[SpeakerTurn]) -> Vec<u32> {
        turns.iter().map(|t| t.speaker).collect()
    }

    #[test]
    fn a_short_interjection_is_absorbed_by_the_surrounding_speaker() {
        let turns = vec![
            turn(0.0, 100.0, 0),
            turn(100.0, 101.0, 1),
            turn(101.0, 200.0, 0),
        ];
        let merged = merge_micro_speakers(turns.clone(), 5.0);
        assert_eq!(speakers(&merged), [0, 0, 0]);
        assert!(
            merged
                .iter()
                .zip(&turns)
                .all(|(m, t)| (m.start, m.end) == (t.start, t.end))
        );
    }

    #[test]
    fn a_trailing_micro_speaker_joins_the_nearest_real_one() {
        let turns = vec![
            turn(0.0, 100.0, 0),
            turn(100.0, 200.0, 1),
            turn(200.0, 202.0, 2),
        ];
        assert_eq!(speakers(&merge_micro_speakers(turns, 5.0)), [0, 1, 1]);
    }

    #[test]
    fn when_everyone_is_micro_the_biggest_speaker_keeps_all() {
        let turns = vec![turn(0.0, 1.0, 0), turn(1.0, 4.0, 1), turn(4.0, 5.0, 2)];
        assert_eq!(speakers(&merge_micro_speakers(turns, 5.0)), [0, 0, 0]);
    }

    #[test]
    fn no_turns_stay_no_turns() {
        assert!(merge_micro_speakers(Vec::new(), 5.0).is_empty());
    }

    /// A file that is not Ogg Opus is refused before any model download (the models dir stays empty,
    /// and the home here does not exist). Spawned: the future must be `Send`, as `serve` needs.
    #[tokio::test]
    async fn non_opus_input_is_refused_before_any_download() {
        let tmp = std::env::temp_dir().join(format!("liberarr-diar-{}", std::process::id()));
        let dir = tmp.join("models");
        std::fs::create_dir_all(&tmp).unwrap();
        let audio = tmp.join("a.wav");
        std::fs::write(&audio, b"RIFF....WAVE").unwrap();
        let diarizer = LocalDiarizer::new(dir.clone());
        let err = tokio::spawn(async move { diarizer.diarize(&audio, None).await })
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{err}");
        assert!(!dir.exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
