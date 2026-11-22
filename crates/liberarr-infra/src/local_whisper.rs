//! Local transcription with whisper.cpp: Silero VAD finds the speech, Whisper (language auto-detected)
//! reads it in chunks of several minutes. Models are downloaded once by [`ModelStore`].
use crate::audio_prep::{decode_opus_16k, normalize_dynamic};
use crate::models::{ModelStore, SILERO_VAD, WHISPER_TURBO};
use liberarr_core::audio::ogg::opus_packets;
use liberarr_core::transcription::{ChunkResult, Segment, Transcriber};
use liberarr_core::{Error, Result};
use std::path::{Path, PathBuf};
use std::sync::Once;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
    WhisperVadContextParams, WhisperVadParams,
};

const ENGINE: &str = "whisper-large-v3-turbo-q5_0";
const RATE: f64 = 16_000.0;
/// A chunk costs a full encoder pass whatever its length, and language detection flips on a short
/// one ("OK", "Oui"): chunks are as long as the speech allows.
const MAX_CHUNK_S: f64 = 300.0;
const MAX_GAP_S: f64 = 10.0;

/// Speech spans (seconds) to chunks: a span joins the current chunk unless the gap is at least
/// [`MAX_GAP_S`] or the chunk would pass [`MAX_CHUNK_S`]; one longer span is cut into pieces.
pub fn speech_chunks(spans: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut chunks: Vec<(f64, f64)> = Vec::new();
    for &(start, end) in spans {
        match chunks.last_mut() {
            Some(last) if start - last.1 < MAX_GAP_S && end - last.0 <= MAX_CHUNK_S => last.1 = end,
            _ => chunks.push((start, end)),
        }
    }
    let mut cut = Vec::new();
    for (mut start, end) in chunks {
        while end - start > MAX_CHUNK_S {
            cut.push((start, start + MAX_CHUNK_S));
            start += MAX_CHUNK_S;
        }
        cut.push((start, end));
    }
    cut
}

pub struct LocalWhisper {
    models: ModelStore,
}

impl LocalWhisper {
    pub fn new(models_dir: PathBuf) -> Self {
        Self {
            models: ModelStore::new(models_dir),
        }
    }

    /// True when a model file is not yet on disk (the first run downloads about 575 MB).
    pub fn models_missing(&self) -> bool {
        !(self.models.is_present(&WHISPER_TURBO) && self.models.is_present(&SILERO_VAD))
    }
}

fn failed(what: &str) -> Error {
    Error::Provider(format!("local transcription: {what}"))
}

/// whisper.cpp is built for fixed instruction sets: on a CPU without them it dies with SIGILL
/// (killing `serve`), so the job fails cleanly instead.
fn check_cpu() -> Result<()> {
    #[cfg(target_arch = "x86_64")]
    let ok = is_x86_feature_detected!("avx")
        && is_x86_feature_detected!("avx2")
        && is_x86_feature_detected!("fma")
        && is_x86_feature_detected!("f16c")
        && is_x86_feature_detected!("bmi2");
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    let ok = std::arch::is_aarch64_feature_detected!("lse")
        && std::arch::is_aarch64_feature_detected!("rdm")
        && std::arch::is_aarch64_feature_detected!("crc");
    #[cfg(not(any(
        target_arch = "x86_64",
        all(target_arch = "aarch64", target_os = "linux")
    )))]
    let ok = true;
    if ok {
        Ok(())
    } else {
        Err(Error::Unsupported(
            "this CPU lacks the instructions local transcription needs (x86_64: AVX2; ARM: Armv8.2-A); use a cloud transcriber"
                .into(),
        ))
    }
}

impl Transcriber for LocalWhisper {
    async fn transcribe(&self, audio: &[u8], _filename: &str) -> Result<ChunkResult> {
        // Cheap check first: a bad file must not trigger the model download.
        if opus_packets(audio).is_err() {
            return Err(Error::Unsupported(
                "local transcription reads Ogg Opus only; use a cloud transcriber for other formats"
                    .into(),
            ));
        }
        check_cpu()?;
        let whisper = self.models.ensure(&WHISPER_TURBO).await?;
        let vad = self.models.ensure(&SILERO_VAD).await?;
        let audio = audio.to_vec();
        tokio::task::spawn_blocking(move || run(&audio, &whisper, &vad))
            .await
            .map_err(|_| failed("worker stopped"))?
    }

    fn engine(&self) -> &str {
        ENGINE
    }
}

/// Decode, normalise, find the speech, then transcribe it. The models live for this call only
/// (about 1 GB that a NAS should not hold between recordings).
fn run(audio: &[u8], whisper: &Path, vad: &Path) -> Result<ChunkResult> {
    // whisper.cpp logs token text in debug builds: route its logs nowhere.
    static HOOKS: Once = Once::new();
    HOOKS.call_once(whisper_rs::install_logging_hooks);

    let mut samples = decode_opus_16k(audio)?;
    normalize_dynamic(&mut samples);

    let vad_path = vad.to_str().ok_or_else(|| failed("bad model path"))?;
    let mut vad = WhisperVadContext::new(vad_path, WhisperVadContextParams::default())
        .map_err(|_| failed("cannot load the VAD model"))?;
    let spans: Vec<(f64, f64)> = vad
        .segments_from_samples(WhisperVadParams::default(), &samples)
        .map_err(|_| failed("speech detection failed"))?
        // Centiseconds.
        .map(|s| (f64::from(s.start) / 100.0, f64::from(s.end) / 100.0))
        .collect();
    drop(vad);
    let chunks = speech_chunks(&spans);
    if chunks.is_empty() {
        return Ok(ChunkResult {
            language: None,
            text: String::new(),
            segments: Vec::new(),
        });
    }

    let whisper_path = whisper.to_str().ok_or_else(|| failed("bad model path"))?;
    let ctx = WhisperContext::new_with_params(whisper_path, WhisperContextParameters::default())
        .map_err(|_| failed("cannot load the Whisper model"))?;
    let mut state = ctx
        .create_state()
        .map_err(|_| failed("cannot start Whisper"))?;
    let threads = std::thread::available_parallelism()
        .map_or(4, usize::from)
        .min(8);

    let mut segments = Vec::new();
    // (chunk length, language): the longest chunk decides the recording's language.
    let mut language: Option<(f64, &'static str)> = None;
    for (start, end) in chunks {
        let from = ((start * RATE) as usize).min(samples.len());
        let to = ((end * RATE) as usize).min(samples.len());
        // Greedy repeated the same words on a noisy stretch of a real recording (12 times); beam search did not, at the same quality and ~20% more time.
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0,
        });
        // whisper-rs defaults to English: "auto" detects the language (French and English mix).
        params.set_language(Some("auto"));
        params.set_translate(false);
        params.set_no_context(true);
        // Past text in the prompt fed repetition loops on real meetings (72 identical segments on a 19-min file): measured fix.
        params.set_n_max_text_ctx(0);
        params.set_suppress_nst(true);
        params.set_n_threads(i32::try_from(threads).unwrap_or(4));
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        state
            .full(params, &samples[from..to])
            .map_err(|_| failed("transcription failed"))?;
        if let Some(lang) = whisper_rs::get_lang_str(state.full_lang_id_from_state())
            && language.is_none_or(|(len, _)| end - start > len)
        {
            language = Some((end - start, lang));
        }
        for seg in state.as_iter() {
            let text = seg.to_str_lossy().map_err(|_| failed("bad text"))?;
            let text = text.trim();
            if !text.is_empty() {
                segments.push(Segment {
                    // Centiseconds, relative to the chunk.
                    start: seg.start_timestamp() as f64 / 100.0 + start,
                    end: seg.end_timestamp() as f64 / 100.0 + start,
                    text: text.to_string(),
                });
            }
        }
    }
    let text = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Ok(ChunkResult {
        language: language.map(|(_, l)| l.to_string()),
        text,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that is not Ogg Opus is refused before any model download (the models dir stays empty,
    /// and the home here does not exist). Spawned: the future must be `Send`, as `serve` needs.
    #[tokio::test]
    async fn non_opus_input_is_refused_before_any_download() {
        let dir = std::env::temp_dir().join(format!("liberarr-no-models-{}", std::process::id()));
        let whisper = LocalWhisper::new(dir.clone());
        let err = tokio::spawn(async move { whisper.transcribe(b"RIFF....WAVE", "a.wav").await })
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)), "{err}");
        assert!(!dir.exists());
    }

    #[test]
    fn no_speech_gives_empty_chunks() {
        assert!(speech_chunks(&[]).is_empty());
    }

    #[test]
    fn chunks_respect_max_len_and_gaps() {
        assert_eq!(
            speech_chunks(&[(0.0, 10.0), (11.0, 20.0), (25.0, 30.0)]),
            [(0.0, 30.0)]
        );
        // A 15 s gap splits.
        assert_eq!(
            speech_chunks(&[(0.0, 100.0), (115.0, 200.0)]),
            [(0.0, 100.0), (115.0, 200.0)]
        );
        // The 300 s cap splits.
        assert_eq!(
            speech_chunks(&[(0.0, 200.0), (201.0, 350.0)]),
            [(0.0, 200.0), (201.0, 350.0)]
        );
        // One long span is cut into 300 s pieces.
        assert_eq!(
            speech_chunks(&[(0.0, 700.0)]),
            [(0.0, 300.0), (300.0, 600.0), (600.0, 700.0)]
        );
        let many: Vec<(f64, f64)> = (0..200)
            .map(|i| (f64::from(i) * 5.0, f64::from(i) * 5.0 + 4.0))
            .collect();
        let chunks = speech_chunks(&many);
        assert!(chunks.windows(2).all(|w| w[0].1 <= w[1].0));
        assert!(chunks.iter().all(|(s, e)| e - s <= 300.0));
    }
}
