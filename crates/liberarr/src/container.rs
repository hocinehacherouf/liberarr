//! Composition root: the one place where adapters are wired to the core ports.
use crate::config::{Config, Profile};
use liberarr_core::ai::{DEFAULT_UPLOAD_BYTES, Provider, Step};
use liberarr_core::exports::ExportToObsidian;
use liberarr_core::jobs::{Job, JobInfo, JobQueue, StepHandler, run_job_once};
use liberarr_core::recordings::{Ingest, Recording, RecordingItem, RecordingRepo};
use liberarr_core::summaries::{
    ChatMessage, Llm, MeetingSummary, Summarize, SummaryRepo, ToggleAction,
};
use liberarr_core::transcription::{
    ChunkResult, Transcribe, Transcriber, Transcript, TranscriptRepo,
};
use liberarr_core::{Error, Result};
use liberarr_infra::ai::OpenAiCompat;
use liberarr_infra::claude_cli::ClaudeCli;
use liberarr_infra::files::{FsAudioStore, FsNoteStore, FsPartialStore, OggSplitter};
use liberarr_infra::keystore::FileKeyStore;
use liberarr_infra::mistral::MistralTranscriber;
use liberarr_infra::sqlite::Db;
use liberarr_web::Backend;
use std::path::PathBuf;

pub struct App {
    pub cfg: Config,
    pub db: Db,
    transcriber: AnyTranscriber,
    llm: AnyLlm,
    audio: FsAudioStore,
}

/// The ports use `async fn` (no `dyn`), so the chosen adapter is an enum.
enum AnyTranscriber {
    Compat(OpenAiCompat),
    Mistral(MistralTranscriber),
}

enum AnyLlm {
    Compat(OpenAiCompat),
    Claude(ClaudeCli),
}

/// Whether the provider documents a vocabulary field: only Mistral (`context_bias`).
fn takes_vocabulary(p: &Profile) -> bool {
    p.provider == Provider::Mistral
}

fn transcriber(p: &Profile, vocabulary: &[String]) -> AnyTranscriber {
    let (url, key, model) = (p.base_url.clone(), p.api_key.clone(), p.model.clone());
    // Elsewhere it is ignored, with a warning at start (`warnings`).
    let vocabulary = if takes_vocabulary(p) {
        vocabulary.to_vec()
    } else {
        Vec::new()
    };
    match p.provider {
        Provider::Mistral => AnyTranscriber::Mistral(
            MistralTranscriber::new(url, key, model).with_context_bias(vocabulary),
        ),
        provider => {
            let t = OpenAiCompat::new(url, key, model);
            AnyTranscriber::Compat(match provider {
                Provider::OpenRouter if !p.routing_only.is_empty() => {
                    t.with_routing(&p.routing_only)
                }
                _ => t,
            })
        }
    }
}

fn llm(p: &Profile) -> AnyLlm {
    let (url, key, model) = (p.base_url.clone(), p.api_key.clone(), p.model.clone());
    match p.provider {
        Provider::Claude => AnyLlm::Claude(ClaudeCli::new(p.model.clone())),
        Provider::OpenRouter if !p.routing_only.is_empty() => {
            AnyLlm::Compat(OpenAiCompat::new(url, key, model).with_routing(&p.routing_only))
        }
        _ => AnyLlm::Compat(OpenAiCompat::new(url, key, model)),
    }
}

impl Transcriber for AnyTranscriber {
    async fn transcribe(&self, audio: &[u8], filename: &str) -> Result<ChunkResult> {
        match self {
            Self::Compat(t) => t.transcribe(audio, filename).await,
            Self::Mistral(t) => t.transcribe(audio, filename).await,
        }
    }

    fn engine(&self) -> &str {
        match self {
            Self::Compat(t) => t.engine(),
            Self::Mistral(t) => t.engine(),
        }
    }
}

impl Llm for AnyLlm {
    async fn complete_json(&self, messages: &[ChatMessage]) -> Result<String> {
        match self {
            Self::Compat(l) => l.complete_json(messages).await,
            Self::Claude(l) => l.complete_json(messages).await,
        }
    }
}

impl App {
    /// Synchronous (`Db::open` may wait on a busy database): call it before entering async code.
    pub fn open(cfg: Config) -> Result<App> {
        std::fs::create_dir_all(&cfg.home)
            .map_err(|e| Error::Storage(format!("create home: {}", e.kind())))?;
        let db = Db::open(&cfg.home.join("liberarr.db"))?;
        Ok(App {
            transcriber: transcriber(&cfg.transcribe, &cfg.vocabulary),
            llm: llm(&cfg.llm),
            audio: FsAudioStore {
                audio: audio_dir(&cfg),
            },
            db,
            cfg,
        })
    }

    pub fn ingest(&self) -> Ingest<'_, Db, Db, FsAudioStore> {
        Ingest {
            recordings: &self.db,
            jobs: &self.db,
            store: &self.audio,
        }
    }

    /// Runs queued jobs until the queue is empty. `Db::reset_stale` must have run once before.
    pub async fn drain(&self) -> Result<()> {
        while run_job_once(&self.db, self).await? {}
        Ok(())
    }
}

/// Configuration warnings to show before work starts: an OpenRouter step without a provider list,
/// a vocabulary the transcription provider cannot take.
pub fn warnings(cfg: &Config) -> Vec<String> {
    let mut w: Vec<String> = [
        (Step::Transcribe, &cfg.transcribe),
        (Step::Summarize, &cfg.llm),
    ]
    .into_iter()
    .filter(|(_, p)| p.provider == Provider::OpenRouter && p.routing_only.is_empty())
    .map(|(step, _)| {
        let var = crate::setup::routing_var(step);
        rust_i18n::t!("config.openrouter_unrestricted", var = var).into_owned()
    })
    .collect();
    if !cfg.vocabulary.is_empty() && !takes_vocabulary(&cfg.transcribe) {
        w.push(rust_i18n::t!("config.vocabulary_ignored").into_owned());
    }
    w
}

/// `<home>/audio`: shared by the partial store (staging) and the audio store (move vs copy rule).
fn audio_dir(cfg: &Config) -> PathBuf {
    cfg.home.join("audio")
}

pub fn keys(cfg: &Config) -> FileKeyStore {
    FileKeyStore::new(cfg.home.join("device").join("keys.bin"), &cfg.secret_key)
}

pub fn partials(cfg: &Config) -> FsPartialStore {
    FsPartialStore {
        dir: cfg.home.join("device").join("partial"),
        raw: cfg.home.join("device").join("raw"),
        audio: audio_dir(cfg),
    }
}

impl StepHandler for App {
    async fn run(&self, job: &Job) -> Result<bool> {
        let id = job.recording_id;
        match job.step.as_str() {
            "transcribe" => Transcribe {
                recordings: &self.db,
                splitter: &OggSplitter,
                transcriber: &self.transcriber,
                transcripts: &self.db,
                max_chunk_bytes: DEFAULT_UPLOAD_BYTES,
            }
            .run(id)
            .await
            .map(|()| true),
            "summarize" => Summarize {
                transcripts: &self.db,
                llm: &self.llm,
                summaries: &self.db,
                lang: self.cfg.content_language,
            }
            .run(id)
            .await
            // No speech: no summary, so the chain stops before the export.
            .map(|s| s.is_some()),
            "export" => ExportToObsidian {
                recordings: &self.db,
                transcripts: &self.db,
                summaries: &self.db,
                notes: &FsNoteStore,
                vault: self.cfg.obsidian.clone(),
                tz: jiff::tz::TimeZone::system(),
                lang: self.cfg.content_language,
            }
            .run(id)
            .await
            .map(|_| true),
            other => Err(Error::Unsupported(format!("unknown step `{other}`"))),
        }
    }
}

impl Backend for App {
    async fn list(&self) -> Result<Vec<RecordingItem>> {
        self.db.list_items().await
    }
    async fn recording(&self, id: i64) -> Result<Option<Recording>> {
        RecordingRepo::get(&self.db, id).await
    }
    async fn transcript(&self, id: i64) -> Result<Option<Transcript>> {
        TranscriptRepo::get(&self.db, id).await
    }
    async fn summary(&self, id: i64) -> Result<Option<MeetingSummary>> {
        SummaryRepo::get(&self.db, id).await
    }
    async fn jobs(&self, id: i64) -> Result<Vec<JobInfo>> {
        self.db.list_for(id).await
    }
    async fn enqueue(&self, id: i64, step: &str) -> Result<i64> {
        self.db.enqueue(id, step).await
    }
    async fn toggle_action(
        &self,
        id: i64,
        index: usize,
        action: &str,
        done: bool,
    ) -> Result<MeetingSummary> {
        ToggleAction {
            summaries: &self.db,
        }
        .run(id, index, action, done)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field the model does not document gets the whole transcription refused: the vocabulary
    /// goes only where it is documented.
    #[test]
    fn vocabulary_only_where_documented() {
        let p = |provider, model: &str| Profile {
            provider,
            base_url: String::new(),
            api_key: String::new(),
            model: model.into(),
            routing_only: Vec::new(),
        };
        assert!(takes_vocabulary(&p(
            Provider::Mistral,
            "voxtral-mini-transcribe-3"
        )));
        for (provider, model) in [
            (Provider::OpenRouter, "openai/gpt-transcribe"),
            (Provider::Compatible, "whisper-1"),
        ] {
            assert!(!takes_vocabulary(&p(provider, model)), "{model}");
        }
    }
}
