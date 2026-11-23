//! Domain and application layer: entities, pure rules, use cases and ports. No I/O.
//!
//! Async ports are native `async fn` in traits, used through generics only (never `dyn`), so the
//! unnameable `Send` bound is resolved at the composition root where the types are concrete.
#![allow(async_fn_in_trait)]

pub mod ai;
pub mod audio;
pub mod device;
pub mod diarization;
pub mod error;
pub mod exports;
pub mod jobs;
pub mod lang;
pub mod recordings;
pub mod search;
pub mod summaries;
pub mod transcription;

pub use error::{Error, Result};

/// In-memory fakes of every port, shared by the inline unit tests.
#[cfg(test)]
mod fakes {
    use crate::diarization::{Diarization, DiarizationRepo, Diarizer, SpeakerTurn};
    use crate::exports::NoteStore;
    use crate::jobs::{Job, JobInfo, JobQueue, JobStatus};
    use crate::recordings::{
        AudioStore, NewRecording, Recording, RecordingItem, RecordingRepo, StoredAudio,
    };
    use crate::summaries::{ChatMessage, Llm, MeetingSummary, SummaryRepo};
    use crate::transcription::{Transcript, TranscriptRepo};
    use crate::{Error, Result};
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, HashMap};
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    /// Fakes never yield, so a busy-poll executor is enough (no async runtime in core).
    pub fn block_on<F: Future>(f: F) -> F::Output {
        let mut cx = Context::from_waker(Waker::noop());
        let mut f = pin!(f);
        loop {
            if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[derive(Default)]
    pub struct MemRecordings(pub RefCell<Vec<Recording>>);
    impl RecordingRepo for MemRecordings {
        async fn get(&self, id: i64) -> Result<Option<Recording>> {
            Ok(self.0.borrow().iter().find(|r| r.id == id).cloned())
        }
        async fn find_by_ref(&self, source_ref: &str) -> Result<Option<i64>> {
            Ok(self
                .0
                .borrow()
                .iter()
                .find(|r| r.source_ref == source_ref)
                .map(|r| r.id))
        }
        async fn add(&self, n: NewRecording) -> Result<i64> {
            let mut all = self.0.borrow_mut();
            let id = all.len() as i64 + 1;
            all.push(Recording {
                id,
                title: n.title,
                source: n.source,
                source_ref: n.source_ref,
                audio_path: n.audio_path,
                duration_s: n.duration_s,
                created_at: n.created_at,
                export_path: None,
            });
            Ok(id)
        }
        async fn set_export_path(&self, id: i64, path: &Path) -> Result<()> {
            let mut all = self.0.borrow_mut();
            let r = all
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or_else(|| Error::NotFound("recording".into()))?;
            r.export_path = Some(path.to_path_buf());
            Ok(())
        }
        async fn list_items(&self) -> Result<Vec<RecordingItem>> {
            unreachable!("not used by core use cases")
        }
    }

    struct Row {
        job: Job,
        status: JobStatus,
        error: Option<String>,
    }

    #[derive(Default)]
    pub struct MemJobs(RefCell<Vec<Row>>);
    impl MemJobs {
        /// (step, status, error) of every job, in enqueue order.
        pub fn rows(&self) -> Vec<(String, JobStatus, Option<String>)> {
            self.0
                .borrow()
                .iter()
                .map(|r| (r.job.step.clone(), r.status, r.error.clone()))
                .collect()
        }
    }
    impl JobQueue for MemJobs {
        async fn enqueue(&self, recording_id: i64, step: &str) -> Result<i64> {
            let mut rows = self.0.borrow_mut();
            let id = rows.len() as i64 + 1;
            rows.push(Row {
                job: Job {
                    id,
                    recording_id,
                    step: step.into(),
                    attempts: 0,
                },
                status: JobStatus::Queued,
                error: None,
            });
            Ok(id)
        }
        async fn claim_next(&self) -> Result<Option<Job>> {
            let mut rows = self.0.borrow_mut();
            let Some(r) = rows.iter_mut().find(|r| r.status == JobStatus::Queued) else {
                return Ok(None);
            };
            r.status = JobStatus::Running;
            r.job.attempts += 1;
            Ok(Some(r.job.clone()))
        }
        async fn complete(&self, job: &Job, then: Option<&str>) -> Result<()> {
            self.0
                .borrow_mut()
                .iter_mut()
                .find(|r| r.job.id == job.id)
                .expect("job")
                .status = JobStatus::Done;
            if let Some(step) = then {
                self.enqueue(job.recording_id, step).await?;
            }
            Ok(())
        }
        async fn fail(&self, job: &Job, error: &str) -> Result<()> {
            let mut rows = self.0.borrow_mut();
            let r = rows.iter_mut().find(|r| r.job.id == job.id).expect("job");
            r.status = JobStatus::Failed;
            r.error = Some(error.into());
            Ok(())
        }
        async fn reset_stale(&self) -> Result<u64> {
            unreachable!("not used by core use cases")
        }
        async fn list_for(&self, recording_id: i64) -> Result<Vec<JobInfo>> {
            Ok(self
                .0
                .borrow()
                .iter()
                .filter(|r| r.job.recording_id == recording_id)
                .map(|r| JobInfo {
                    id: r.job.id,
                    step: r.job.step.clone(),
                    status: r.status,
                    attempts: r.job.attempts,
                    error: r.error.clone(),
                    updated_at: jiff::Timestamp::UNIX_EPOCH,
                })
                .collect())
        }
    }

    #[derive(Default)]
    pub struct MemTranscripts {
        pub map: RefCell<HashMap<i64, Transcript>>,
        pub saves: Cell<u32>,
    }
    impl TranscriptRepo for MemTranscripts {
        async fn save(&self, recording_id: i64, t: &Transcript) -> Result<()> {
            self.saves.set(self.saves.get() + 1);
            self.map.borrow_mut().insert(recording_id, t.clone());
            Ok(())
        }
        async fn get(&self, recording_id: i64) -> Result<Option<Transcript>> {
            Ok(self.map.borrow().get(&recording_id).cloned())
        }
    }

    #[derive(Default)]
    pub struct MemSummaries(pub RefCell<HashMap<i64, MeetingSummary>>);
    impl SummaryRepo for MemSummaries {
        async fn save(&self, recording_id: i64, s: &MeetingSummary) -> Result<()> {
            self.0.borrow_mut().insert(recording_id, s.clone());
            Ok(())
        }
        async fn get(&self, recording_id: i64) -> Result<Option<MeetingSummary>> {
            Ok(self.0.borrow().get(&recording_id).cloned())
        }
        async fn set_action_done(
            &self,
            recording_id: i64,
            index: usize,
            action: &str,
            done: bool,
        ) -> Result<bool> {
            let mut map = self.0.borrow_mut();
            let a = map
                .get_mut(&recording_id)
                .and_then(|s| s.actions.get_mut(index))
                .filter(|a| a.action == action);
            Ok(a.map(|a| a.done = done).is_some())
        }
        async fn open_actions(&self) -> Result<u32> {
            let map = self.0.borrow();
            Ok(map
                .values()
                .flat_map(|s| &s.actions)
                .map(|a| u32::from(!a.done))
                .sum())
        }
    }

    /// Replies with the scripted answers in order and counts calls.
    pub struct FakeLlm {
        pub replies: RefCell<Vec<String>>,
        pub calls: Cell<u32>,
    }
    impl FakeLlm {
        pub fn new(replies: &[&str]) -> Self {
            Self {
                replies: RefCell::new(replies.iter().rev().map(|s| s.to_string()).collect()),
                calls: Cell::new(0),
            }
        }
    }
    impl Llm for FakeLlm {
        async fn complete_json(&self, _messages: &[ChatMessage]) -> Result<String> {
            self.calls.set(self.calls.get() + 1);
            self.replies
                .borrow_mut()
                .pop()
                .ok_or_else(|| Error::Provider("no scripted reply".into()))
        }
    }

    #[derive(Default)]
    pub struct MemDiarizations {
        pub saved: RefCell<HashMap<i64, Diarization>>,
        pub requested: RefCell<HashMap<i64, u32>>,
        pub names: RefCell<HashMap<(i64, u32), String>>,
    }
    impl DiarizationRepo for MemDiarizations {
        async fn get(&self, recording_id: i64) -> Result<Option<Diarization>> {
            Ok(self.saved.borrow().get(&recording_id).cloned())
        }
        async fn save(&self, recording_id: i64, d: &Diarization) -> Result<()> {
            self.saved.borrow_mut().insert(recording_id, d.clone());
            Ok(())
        }
        async fn requested_speakers(&self, recording_id: i64) -> Result<Option<u32>> {
            Ok(self.requested.borrow().get(&recording_id).copied())
        }
        async fn set_requested_speakers(&self, recording_id: i64, n: Option<u32>) -> Result<()> {
            match n {
                Some(n) => self.requested.borrow_mut().insert(recording_id, n),
                None => self.requested.borrow_mut().remove(&recording_id),
            };
            Ok(())
        }
        async fn names(&self, recording_id: i64) -> Result<BTreeMap<u32, String>> {
            Ok(self
                .names
                .borrow()
                .iter()
                .filter(|((id, _), _)| *id == recording_id)
                .map(|((_, speaker), name)| (*speaker, name.clone()))
                .collect())
        }
        async fn set_name(
            &self,
            recording_id: i64,
            speaker: u32,
            name: Option<&str>,
        ) -> Result<()> {
            let key = (recording_id, speaker);
            match name.map(str::trim).filter(|n| !n.is_empty()) {
                Some(n) => self.names.borrow_mut().insert(key, n.into()),
                None => self.names.borrow_mut().remove(&key),
            };
            Ok(())
        }
    }

    /// One speaker over the first two seconds; counts calls and keeps the last requested count.
    /// `unsupported` makes it refuse the audio, like the real one does for non-Ogg files.
    #[derive(Default)]
    pub struct FakeDiarizer {
        pub calls: Cell<u32>,
        pub last_count: Cell<Option<u32>>,
        pub unsupported: bool,
    }
    impl Diarizer for FakeDiarizer {
        async fn diarize(
            &self,
            _audio_path: &Path,
            speakers: Option<u32>,
        ) -> Result<Vec<SpeakerTurn>> {
            self.calls.set(self.calls.get() + 1);
            self.last_count.set(speakers);
            if self.unsupported {
                return Err(crate::Error::Unsupported("fake".into()));
            }
            Ok(vec![SpeakerTurn {
                start: 0.0,
                end: 2.0,
                speaker: 0,
            }])
        }
        fn engine(&self) -> &str {
            "fake"
        }
    }

    #[derive(Default)]
    pub struct MemNotes(pub RefCell<HashMap<PathBuf, String>>);
    impl NoteStore for MemNotes {
        fn read(&self, path: &Path) -> Result<Option<String>> {
            Ok(self.0.borrow().get(path).cloned())
        }
        fn write_atomic(&self, path: &Path, text: &str) -> Result<()> {
            self.0.borrow_mut().insert(path.to_path_buf(), text.into());
            Ok(())
        }
    }

    /// "Hashes" a file to its file name, so two paths with the same name are the same audio.
    pub struct FakeStore;
    impl AudioStore for FakeStore {
        fn fingerprint(&self, src: &Path) -> Result<String> {
            Ok(src
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned())
        }
        fn store(&self, _src: &Path, fingerprint: &str) -> Result<StoredAudio> {
            Ok(StoredAudio {
                path: Path::new("/audio").join(fingerprint),
                duration_s: Some(1.0),
            })
        }
    }
}
