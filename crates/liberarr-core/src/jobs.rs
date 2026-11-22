//! Jobs context: resumable pipeline steps, a queue port and the one-job runner.
use crate::Result;

/// The pipeline, in order. A step's success enqueues the next one.
pub const STEPS: [&str; 4] = ["transcribe", "summarize", "diarize", "export"];
/// Steps whose failure is recorded but does not stop the chain: the export still runs.
pub const OPTIONAL_STEPS: [&str; 1] = ["diarize"];
pub const MAX_ERROR: usize = 2000;

pub fn next_step(step: &str) -> Option<&'static str> {
    let i = STEPS.iter().position(|s| *s == step)?;
    STEPS.get(i + 1).copied()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: i64,
    pub recording_id: i64,
    pub step: String,
    pub attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Failed,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInfo {
    pub id: i64,
    pub step: String,
    pub status: JobStatus,
    pub attempts: u32,
    pub error: Option<String>,
    pub updated_at: jiff::Timestamp,
}

pub trait JobQueue {
    async fn enqueue(&self, recording_id: i64, step: &str) -> Result<i64>;
    /// Atomic: two workers never get the same job.
    async fn claim_next(&self) -> Result<Option<Job>>;
    /// Marks the job done and, in the same transaction, enqueues `then`.
    async fn complete(&self, job: &Job, then: Option<&str>) -> Result<()>;
    /// `error` is at most `MAX_ERROR` chars (the runner truncates).
    async fn fail(&self, job: &Job, error: &str) -> Result<()>;
    /// Re-queues jobs left "running" by a dead process.
    async fn reset_stale(&self) -> Result<u64>;
    async fn list_for(&self, recording_id: i64) -> Result<Vec<JobInfo>>;
}

/// A recording with a queued or running job: a second run would call the AI twice.
pub fn busy(jobs: &[JobInfo]) -> bool {
    jobs.iter()
        .any(|j| matches!(j.status, JobStatus::Queued | JobStatus::Running))
}

/// Whether a save must queue an `export`: not while a job is queued or a non-export step is running, as such a
/// chain ends with an export that reads the change. A running export may have read the old text.
pub fn export_needed(jobs: &[JobInfo]) -> bool {
    // shortcut: a chain that stops on a failed required step never exports; a rerun recovers it
    !jobs.iter().any(|j| match j.status {
        JobStatus::Queued => true,
        JobStatus::Running => j.step != "export",
        _ => false,
    })
}

/// The latest job of each step, in queue order: where each step stands now. `jobs` is in queue
/// order, as `JobQueue::list_for` returns it.
pub fn latest_by_step(jobs: &[JobInfo]) -> Vec<&JobInfo> {
    jobs.iter()
        .enumerate()
        .filter(|(i, j)| !jobs[i + 1..].iter().any(|l| l.step == j.step))
        .map(|(_, j)| j)
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Rerun {
    Queued(String),
    Busy,
    NothingFailed,
}

/// Queues `step`, or the failed step when `None`. Nothing while the recording is busy. The step
/// overwrites prior outputs only when it succeeds.
pub async fn rerun<Q: JobQueue>(q: &Q, recording_id: i64, step: Option<&str>) -> Result<Rerun> {
    // shortcut: check-then-enqueue is not atomic; two simultaneous reruns could both pass.
    let jobs = q.list_for(recording_id).await?;
    if busy(&jobs) {
        return Ok(Rerun::Busy);
    }
    let step = match step {
        Some(s) => s.to_string(),
        // The latest failure that no later success of the same step has replaced. Not just the
        // last job: a failed optional step is followed by the jobs after it.
        None => match jobs.iter().enumerate().rev().find(|(i, j)| {
            j.status == JobStatus::Failed
                && !jobs[i + 1..]
                    .iter()
                    .any(|l| l.step == j.step && l.status == JobStatus::Done)
        }) {
            Some((_, j)) => j.step.clone(),
            None => return Ok(Rerun::NothingFailed),
        },
    };
    q.enqueue(recording_id, &step).await?;
    Ok(Rerun::Queued(step))
}

/// Runs one pipeline step for a job; the composition root dispatches on `job.step`.
pub trait StepHandler {
    /// `Ok(false)` ends the chain there (e.g. no speech: nothing to summarize or export).
    async fn run(&self, job: &Job) -> Result<bool>;
}

/// Claims and runs one job. `Ok(false)` when the queue is empty. A handler error is recorded on
/// the job (prior outputs stay) and is not an error of the runner itself.
pub async fn run_job_once<Q: JobQueue, H: StepHandler>(q: &Q, handlers: &H) -> Result<bool> {
    let Some(job) = q.claim_next().await? else {
        return Ok(false);
    };
    match handlers.run(&job).await {
        Ok(go_on) => {
            q.complete(&job, next_step(&job.step).filter(|_| go_on))
                .await?
        }
        Err(e) => {
            let msg: String = e.to_string().chars().take(MAX_ERROR).collect();
            q.fail(&job, &msg).await?;
            if OPTIONAL_STEPS.contains(&job.step.as_str())
                && let Some(next) = next_step(&job.step)
            {
                // shortcut: a crash between fail and enqueue stops the chain; a rerun recovers it
                q.enqueue(job.recording_id, next).await?;
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::fakes::{MemJobs, block_on};

    /// Fails the chosen step, succeeds on the others.
    struct FailAt(&'static str);
    impl StepHandler for FailAt {
        async fn run(&self, job: &Job) -> Result<bool> {
            if job.step == self.0 {
                Err(Error::Provider("boom".into()))
            } else {
                Ok(true)
            }
        }
    }

    #[test]
    fn job_chain_and_failure() {
        let q = MemJobs::default();
        let h = FailAt("summarize");
        block_on(q.enqueue(1, "transcribe")).unwrap();

        // Success enqueues the next step.
        assert!(block_on(run_job_once(&q, &h)).unwrap());
        assert_eq!(q.rows()[1].0, "summarize");

        // Failure records the error and stops the chain: nothing is enqueued after it.
        assert!(block_on(run_job_once(&q, &h)).unwrap());
        let rows = q.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].1, JobStatus::Done);
        assert_eq!(rows[1].1, JobStatus::Failed);
        assert_eq!(rows[1].2.as_deref(), Some("provider: boom"));
        assert!(!block_on(run_job_once(&q, &h)).unwrap());
    }

    #[test]
    fn latest_by_step_keeps_the_newest_job_of_each_step() {
        let job = |id, step: &str, status| JobInfo {
            id,
            step: step.into(),
            status,
            attempts: 1,
            error: None,
            updated_at: jiff::Timestamp::UNIX_EPOCH,
        };
        let jobs = [
            job(1, "diarize", JobStatus::Failed),
            job(2, "export", JobStatus::Done),
            job(3, "diarize", JobStatus::Done),
            job(4, "summarize", JobStatus::Failed),
        ];
        let latest: Vec<_> = latest_by_step(&jobs).iter().map(|j| j.id).collect();
        assert_eq!(latest, [2, 3, 4]);
    }

    #[test]
    fn export_needed_unless_a_later_export_will_read_the_change() {
        let job = |step: &str, status| JobInfo {
            id: 1,
            step: step.into(),
            status,
            attempts: 1,
            error: None,
            updated_at: jiff::Timestamp::UNIX_EPOCH,
        };
        assert!(export_needed(&[]));
        assert!(export_needed(&[
            job("export", JobStatus::Done),
            job("diarize", JobStatus::Failed)
        ]));
        // A running export may have read the old text.
        assert!(export_needed(&[job("export", JobStatus::Running)]));
        assert!(!export_needed(&[job("export", JobStatus::Queued)]));
        assert!(!export_needed(&[job("diarize", JobStatus::Running)]));
        assert!(!export_needed(&[job("transcribe", JobStatus::Queued)]));
    }

    #[test]
    fn rerun_failed_step_once() {
        let q = MemJobs::default();
        block_on(q.enqueue(1, "transcribe")).unwrap();
        assert_eq!(block_on(rerun(&q, 1, None)).unwrap(), Rerun::Busy);
        block_on(run_job_once(&q, &FailAt("transcribe"))).unwrap();

        assert_eq!(
            block_on(rerun(&q, 1, None)).unwrap(),
            Rerun::Queued("transcribe".into())
        );
        // No double enqueue while the rerun is queued.
        assert_eq!(block_on(rerun(&q, 1, Some("export"))).unwrap(), Rerun::Busy);
        assert_eq!(q.rows().len(), 2);
        // Once the rerun succeeds, nothing is left failed.
        block_on(run_job_once(&q, &FailAt("none"))).unwrap();
        assert_eq!(q.rows().len(), 3, "success chains to summarize");
        for _ in 0..3 {
            block_on(run_job_once(&q, &FailAt("none"))).unwrap();
        }
        assert_eq!(block_on(rerun(&q, 1, None)).unwrap(), Rerun::NothingFailed);
    }

    #[test]
    fn optional_step_failure_continues_the_chain() {
        let q = MemJobs::default();
        let h = FailAt("diarize");
        block_on(q.enqueue(1, "diarize")).unwrap();
        block_on(run_job_once(&q, &h)).unwrap();
        let rows = q.rows();
        assert_eq!(rows[0].1, JobStatus::Failed);
        assert_eq!(
            (rows[1].0.as_str(), rows[1].1),
            ("export", JobStatus::Queued)
        );

        // A required step still stops the chain.
        let q = MemJobs::default();
        block_on(q.enqueue(1, "summarize")).unwrap();
        block_on(run_job_once(&q, &FailAt("summarize"))).unwrap();
        assert_eq!(q.rows().len(), 1);
    }

    #[test]
    fn rerun_none_picks_a_failed_optional_step() {
        let q = MemJobs::default();
        let h = FailAt("diarize");
        block_on(q.enqueue(1, "transcribe")).unwrap();
        for _ in 0..4 {
            block_on(run_job_once(&q, &h)).unwrap();
        }
        let steps: Vec<_> = q.rows().into_iter().map(|r| (r.0, r.1)).collect();
        assert_eq!(
            steps,
            [
                ("transcribe".to_string(), JobStatus::Done),
                ("summarize".to_string(), JobStatus::Done),
                ("diarize".to_string(), JobStatus::Failed),
                ("export".to_string(), JobStatus::Done),
            ]
        );
        assert_eq!(
            block_on(rerun(&q, 1, None)).unwrap(),
            Rerun::Queued("diarize".into())
        );
        // Once the rerun succeeds (and the export after it), nothing is left failed.
        for _ in 0..2 {
            block_on(run_job_once(&q, &FailAt("none"))).unwrap();
        }
        assert_eq!(block_on(rerun(&q, 1, None)).unwrap(), Rerun::NothingFailed);
    }
}
