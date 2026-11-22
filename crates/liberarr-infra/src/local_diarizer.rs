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
/// 21 speakers with TitaNet; 0.85 gave the right count on every labelled test file. Ignored when the
/// owner set the number of speakers.
const CLUSTER_THRESHOLD: f32 = 0.85;

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

impl Diarizer for LocalDiarizer {
    async fn diarize(&self, audio_path: &Path, speakers: Option<u32>) -> Result<Vec<SpeakerTurn>> {
        let audio = std::fs::read(audio_path).map_err(|_| failed("cannot read the audio"))?;
        // Cheap check first: a bad file must not trigger the model download. No CPU check: onnxruntime
        // picks its kernels at run time, so this works where whisper.cpp does not.
        if opus_packets(&audio).is_err() {
            return Err(Error::Unsupported(
                "local diarization reads Ogg Opus only".into(),
            ));
        }
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
    Ok(renumber(&segments))
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
