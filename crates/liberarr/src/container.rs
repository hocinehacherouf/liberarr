//! Composition root: the one place where adapters are wired to the core ports.
use crate::config::{Config, Profile};
use liberarr_core::ai::{DEFAULT_UPLOAD_BYTES, Provider, Step};
use liberarr_core::diarization::{Diarization, DiarizationRepo, Diarize};
use liberarr_core::exports::ExportToObsidian;
use liberarr_core::jobs::{Job, JobInfo, JobQueue, StepHandler, run_job_once};
use liberarr_core::recordings::{Ingest, Recording, RecordingItem, RecordingRepo};
use liberarr_core::search::{TranscriptHit, search_transcripts};
use liberarr_core::summaries::{
    ChatMessage, Llm, MeetingSummary, RecordingActions, Summarize, SummaryEditRepo, SummaryRepo,
    SummaryText, ToggleAction, actions_by_recording,
};
use liberarr_core::transcription::{
    ChunkResult, SegmentChange, SegmentEdit, Transcribe, Transcriber, Transcript,
    TranscriptEditRepo, TranscriptRepo,
};
use liberarr_core::{Error, Result};
use liberarr_infra::agent_cli::{AgentCli, Preset};
use liberarr_infra::ai::OpenAiCompat;
use liberarr_infra::files::{FsAudioStore, FsNoteStore, FsPartialStore, OggSplitter};
use liberarr_infra::keystore::FileKeyStore;
use liberarr_infra::local_diarizer::LocalDiarizer;
use liberarr_infra::local_whisper::LocalWhisper;
use liberarr_infra::mistral::MistralTranscriber;
use liberarr_infra::sqlite::Db;
use liberarr_web::Backend;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct App {
    pub cfg: Config,
    pub db: Db,
    transcriber: AnyTranscriber,
    /// `None`: no summary engine, the summarize step passes without doing anything.
    llm: Option<AnyLlm>,
    diarizer: LocalDiarizer,
    audio: FsAudioStore,
}

/// The ports use `async fn` (no `dyn`), so the chosen adapter is an enum.
enum AnyTranscriber {
    Compat(OpenAiCompat),
    Mistral(MistralTranscriber),
    Local(LocalWhisper),
}

enum AnyLlm {
    Compat(OpenAiCompat),
    Cli(AgentCli),
}

/// Whether the provider takes a vocabulary: only Mistral (`context_bias`).
fn takes_vocabulary(p: &Profile) -> bool {
    p.provider == Provider::Mistral
}

fn transcriber(p: &Profile, vocabulary: &[String], home: &Path) -> AnyTranscriber {
    let (url, key, model) = (p.base_url.clone(), p.api_key.clone(), p.model.clone());
    // Elsewhere it is ignored, with a warning at start (`warnings`).
    let vocabulary = if takes_vocabulary(p) {
        vocabulary.to_vec()
    } else {
        Vec::new()
    };
    match p.provider {
        Provider::Local => AnyTranscriber::Local(LocalWhisper::new(home.join("models"))),
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

fn llm(p: &Profile, claude_oauth_token: Option<&str>) -> AnyLlm {
    if let Some(preset) = Preset::from_provider(p.provider) {
        return AnyLlm::Cli(AgentCli::new(
            preset,
            p.program.clone(),
            p.model.clone(),
            claude_oauth_token.map(String::from),
        ));
    }
    let (url, key, model) = (p.base_url.clone(), p.api_key.clone(), p.model.clone());
    match p.provider {
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
            Self::Local(t) => t.transcribe(audio, filename).await,
        }
    }

    fn engine(&self) -> &str {
        match self {
            Self::Compat(t) => t.engine(),
            Self::Mistral(t) => t.engine(),
            Self::Local(t) => t.engine(),
        }
    }
}

impl Llm for AnyLlm {
    async fn complete_json(&self, messages: &[ChatMessage]) -> Result<String> {
        match self {
            Self::Compat(l) => l.complete_json(messages).await,
            Self::Cli(l) => l.complete_json(messages).await,
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
            transcriber: transcriber(&cfg.transcribe, &cfg.vocabulary, &cfg.home),
            llm: cfg
                .llm
                .as_ref()
                .map(|p| llm(p, cfg.claude_oauth_token.as_deref())),
            diarizer: LocalDiarizer::new(cfg.home.join("models")),
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

    /// One line when the first run will download a local model (Whisper, the speaker models, or both).
    pub fn model_download_notice(&self) -> Option<String> {
        let whisper = matches!(&self.transcriber, AnyTranscriber::Local(t) if t.models_missing());
        let speakers = self.cfg.diarize && self.diarizer.models_missing();
        let key = match (whisper, speakers) {
            (true, true) => "models.download_notice",
            (true, false) => "transcribe.model_download_notice",
            (false, true) => "diarize.model_download_notice",
            (false, false) => return None,
        };
        Some(rust_i18n::t!(key).into_owned())
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
        Some((Step::Transcribe, &cfg.transcribe)),
        cfg.llm.as_ref().map(|p| (Step::Summarize, p)),
    ]
    .into_iter()
    .flatten()
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
                // Local: the whole file is one chunk, `LocalWhisper` chunks by speech itself.
                max_chunk_bytes: if self.cfg.transcribe.provider == Provider::Local {
                    usize::MAX
                } else {
                    DEFAULT_UPLOAD_BYTES
                },
            }
            .run(id)
            .await
            .map(|()| true),
            // No engine: nothing to call, the export still writes the note.
            "summarize" => match &self.llm {
                None => Ok(true),
                Some(llm) => Summarize {
                    transcripts: &self.db,
                    llm,
                    summaries: &self.db,
                    lang: self.cfg.content_language,
                }
                .run(id)
                .await
                // No speech: no summary, so the chain stops before the export.
                .map(|s| s.is_some()),
            },
            // Off: nothing to run, the chain goes on.
            "diarize" if !self.cfg.diarize => Ok(true),
            "diarize" => Diarize {
                recordings: &self.db,
                transcripts: &self.db,
                diarizer: &self.diarizer,
                store: &self.db,
            }
            .run(id)
            .await
            .map(|()| true),
            "export" => ExportToObsidian {
                recordings: &self.db,
                transcripts: &self.db,
                summaries: &self.db,
                speakers: &self.db,
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
    async fn actions(&self) -> Result<Vec<RecordingActions>> {
        actions_by_recording(&self.db, &self.db).await
    }
    async fn search(&self, query: &str, max: usize) -> Result<Vec<TranscriptHit>> {
        search_transcripts(&self.db, &self.db.list_items().await?, query, max).await
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
    async fn transcript_with_edits(
        &self,
        id: i64,
    ) -> Result<Option<(Transcript, Vec<SegmentEdit>)>> {
        self.db.get_with_edits(id).await
    }
    async fn summary_with_edit(
        &self,
        id: i64,
    ) -> Result<Option<(MeetingSummary, Option<SummaryText>)>> {
        self.db.get_with_edit(id).await
    }
    async fn set_transcript_edits(&self, id: i64, changes: &[SegmentChange]) -> Result<()> {
        self.db.set_edits(id, changes).await
    }
    async fn set_summary_edit(
        &self,
        id: i64,
        generated: &SummaryText,
        current: &SummaryText,
        edit: &SummaryText,
    ) -> Result<()> {
        self.db.set_edit(id, generated, current, edit).await
    }
    async fn jobs(&self, id: i64) -> Result<Vec<JobInfo>> {
        self.db.list_for(id).await
    }
    async fn enqueue(&self, id: i64, step: &str) -> Result<i64> {
        if step == "summarize" && self.llm.is_none() {
            return Err(Error::Conflict("summary engine".into()));
        }
        if step == "diarize" && !self.cfg.diarize {
            return Err(Error::Conflict("diarization".into()));
        }
        self.db.enqueue(id, step).await
    }
    async fn diarization(&self, id: i64) -> Result<Option<Diarization>> {
        DiarizationRepo::get(&self.db, id).await
    }
    async fn speaker_names(&self, id: i64) -> Result<BTreeMap<u32, String>> {
        self.db.names(id).await
    }
    async fn requested_speakers(&self, id: i64) -> Result<Option<u32>> {
        self.db.requested_speakers(id).await
    }
    async fn set_speaker_name(&self, id: i64, speaker: u32, name: Option<&str>) -> Result<()> {
        self.db.set_name(id, speaker, name).await
    }
    async fn set_speaker_count(&self, id: i64, count: Option<u32>) -> Result<()> {
        if !self.cfg.diarize {
            return Err(Error::Conflict("diarization".into()));
        }
        self.db.set_requested_speakers(id, count).await
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
    use liberarr_core::jobs::JobStatus;
    use liberarr_core::lang::Language;
    use liberarr_core::recordings::{NewRecording, Source};
    use liberarr_core::transcription::Segment;

    fn profile() -> Profile {
        Profile {
            provider: Provider::Compatible,
            base_url: String::new(),
            api_key: String::new(),
            model: "m".into(),
            routing_only: Vec::new(),
            program: None,
        }
    }

    /// A home of its own (tests run in parallel), a runtime and an app without a summary engine.
    fn open(name: &str, diarize: bool) -> (PathBuf, tokio::runtime::Runtime, App) {
        let home = std::env::temp_dir().join(format!(
            "liberarr-{name}-{}-{}",
            std::process::id(),
            jiff::Timestamp::now().as_nanosecond()
        ));
        let app = App::open(Config {
            home: home.clone(),
            secret_key: "k".repeat(32),
            api_key: String::new(),
            admin_password: String::new(),
            transcribe: profile(),
            llm: None,
            vocabulary: Vec::new(),
            obsidian: home.join("notes"),
            content_language: Language::En,
            content_language_set: false,
            cookie_secure: false,
            claude_oauth_token: None,
            diarize,
        })
        .unwrap();
        (home, tokio::runtime::Runtime::new().unwrap(), app)
    }

    /// A recording with a one-segment transcript and a queued summarize job; its audio is missing.
    fn recording(rt: &tokio::runtime::Runtime, app: &App, title: &str, text: &str) -> i64 {
        let id = rt
            .block_on(app.db.add(NewRecording {
                title: Some(title.into()),
                source: Source::File,
                source_ref: title.into(),
                audio_path: "a.ogg".into(),
                duration_s: Some(10.0),
                created_at: "2026-03-02T10:00:00Z".parse().unwrap(),
            }))
            .unwrap();
        rt.block_on(TranscriptRepo::save(
            &app.db,
            id,
            &Transcript {
                engine: "e".into(),
                language: None,
                text: text.into(),
                segments: vec![Segment {
                    start: 0.0,
                    end: 2.0,
                    text: text.into(),
                }],
                has_timestamps: true,
            },
        ))
        .unwrap();
        rt.block_on(app.db.enqueue(id, "summarize")).unwrap();
        id
    }

    /// Without an engine the summarize job completes, the note is still written for a transcript
    /// with speech, and the API refuses to queue a summarize that could not run. With diarization
    /// off the diarize job passes too, and the API refuses to queue it or to set a speaker count.
    #[test]
    fn no_engine_skips_summarize_and_refuses_rerun() {
        let (home, rt, app) = open("noengine", false);
        let spoken = recording(&rt, &app, "Budget review", "Bonjour, let's start.");
        let silent = recording(&rt, &app, "Silence", "  ");
        rt.block_on(app.drain()).unwrap();

        for id in [spoken, silent] {
            let jobs = rt.block_on(app.db.list_for(id)).unwrap();
            let steps: Vec<_> = jobs.iter().map(|j| (j.step.as_str(), j.status)).collect();
            assert_eq!(
                steps,
                [
                    ("summarize", JobStatus::Done),
                    ("diarize", JobStatus::Done),
                    ("export", JobStatus::Done)
                ]
            );
        }
        let note = |id| {
            rt.block_on(RecordingRepo::get(&app.db, id))
                .unwrap()
                .unwrap()
                .export_path
        };
        assert!(home.join("notes").join(note(spoken).unwrap()).is_file());
        assert!(note(silent).is_none());

        let err = rt.block_on(Backend::enqueue(&app, spoken, "summarize"));
        assert!(matches!(err, Err(Error::Conflict(_))), "{err:?}");
        assert!(
            rt.block_on(Backend::enqueue(&app, spoken, "export"))
                .is_ok()
        );
        let err = rt.block_on(Backend::enqueue(&app, spoken, "diarize"));
        assert!(matches!(err, Err(Error::Conflict(_))), "{err:?}");
        let err = rt.block_on(Backend::set_speaker_count(&app, spoken, Some(2)));
        assert!(matches!(err, Err(Error::Conflict(_))), "{err:?}");
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// The search reads the corrected transcript: an edited name is found, a removed segment is not.
    #[test]
    fn search_reads_the_corrected_transcript() {
        let (home, rt, app) = open("search", false);
        let id = recording(&rt, &app, "Budget review", "x");
        let seg = |start, end, text: &str| Segment {
            start,
            end,
            text: text.into(),
        };
        let segments = vec![
            seg(0.0, 4.0, "Bonjour Hélène."),
            seg(5.0, 9.0, "Le budget secret."),
            seg(10.0, 12.0, "Fin."),
        ];
        rt.block_on(TranscriptRepo::save(
            &app.db,
            id,
            &Transcript {
                engine: "e".into(),
                language: None,
                text: "Bonjour Hélène. Le budget secret. Fin.".into(),
                segments,
                has_timestamps: true,
            },
        ))
        .unwrap();
        let change = |index, current: &str, text: &str| SegmentChange {
            index,
            current: current.into(),
            text: text.into(),
        };
        rt.block_on(TranscriptEditRepo::set_edits(
            &app.db,
            id,
            &[
                change(0, "Bonjour Hélène.", "Bonjour Hélène Martin."),
                change(1, "Le budget secret.", ""),
            ],
        ))
        .unwrap();
        let hits = rt.block_on(Backend::search(&app, "martin", 10)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (
                hits[0].recording_id,
                hits[0].start,
                hits[0].excerpt.hit.as_str()
            ),
            (id, Some(0.0), "Martin")
        );
        assert!(
            rt.block_on(Backend::search(&app, "secret", 10))
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A diarization that fails (here: the audio is missing) is recorded on its job and shows in the
    /// recording's status, but the export still writes the note.
    #[test]
    fn failed_diarization_does_not_stop_the_note() {
        let (home, rt, app) = open("diarfail", true);
        let id = recording(&rt, &app, "Budget review", "Bonjour, let's start.");
        rt.block_on(app.drain()).unwrap();

        let jobs = rt.block_on(app.db.list_for(id)).unwrap();
        let steps: Vec<_> = jobs.iter().map(|j| (j.step.as_str(), j.status)).collect();
        assert_eq!(
            steps,
            [
                ("summarize", JobStatus::Done),
                ("diarize", JobStatus::Failed),
                ("export", JobStatus::Done)
            ]
        );
        let item = &rt.block_on(app.db.list_items()).unwrap()[0];
        assert_eq!(item.failed_step.as_deref(), Some("diarize"));
        let note = rt
            .block_on(RecordingRepo::get(&app.db, id))
            .unwrap()
            .unwrap();
        assert!(home.join("notes").join(note.export_path.unwrap()).is_file());
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A field the model does not document gets the whole transcription refused: the vocabulary
    /// goes only where it is documented.
    #[test]
    fn vocabulary_only_where_documented() {
        let p = |provider, model: &str| Profile {
            provider,
            model: model.into(),
            ..profile()
        };
        assert!(takes_vocabulary(&p(
            Provider::Mistral,
            "voxtral-mini-transcribe-3"
        )));
        assert!(!takes_vocabulary(&p(Provider::Local, "")));
        for (provider, model) in [
            (Provider::OpenRouter, "openai/gpt-transcribe"),
            (Provider::Compatible, "whisper-1"),
        ] {
            assert!(!takes_vocabulary(&p(provider, model)), "{model}");
        }
    }
}
