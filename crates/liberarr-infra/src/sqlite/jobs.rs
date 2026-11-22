//! `JobQueue` on SQLite. Claims are atomic across processes.
use super::{Db, now, parse_ts, storage, write_tx};
use liberarr_core::jobs::{Job, JobInfo, JobQueue, JobStatus};
use liberarr_core::{Error, Result};
use rusqlite::{OptionalExtension, params};
use std::fs::TryLockError;
use std::sync::Arc;

fn parse_status(s: &str) -> rusqlite::Result<JobStatus> {
    Ok(match s {
        "queued" => JobStatus::Queued,
        "running" => JobStatus::Running,
        "failed" => JobStatus::Failed,
        "done" => JobStatus::Done,
        _ => {
            return Err(rusqlite::Error::InvalidColumnType(
                2,
                "status".into(),
                rusqlite::types::Type::Text,
            ));
        }
    })
}

/// A stale worker finishing a job that was re-queued (or already finished) must not go through.
fn expect_running(changed: usize) -> Result<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(Error::Storage(
            "job is not running (re-queued or already finished)".into(),
        ))
    }
}

impl JobQueue for Db {
    async fn enqueue(&self, recording_id: i64, step: &str) -> Result<i64> {
        let step = step.to_owned();
        self.run(move |c| {
            let t = now();
            c.execute(
                "INSERT INTO jobs (recording_id, step, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                params![recording_id, step, t],
            )?;
            Ok(c.last_insert_rowid())
        })
        .await
    }

    async fn claim_next(&self) -> Result<Option<Job>> {
        self.run(|c| {
            // IMMEDIATE + a single UPDATE … RETURNING: select and claim cannot be interleaved.
            let tx = write_tx(c)?;
            let job = tx
                .query_row(
                    "UPDATE jobs SET status = 'running', attempts = attempts + 1, updated_at = ?1
                     WHERE id = (SELECT id FROM jobs WHERE status = 'queued' ORDER BY id LIMIT 1)
                     RETURNING id, recording_id, step, attempts",
                    [now()],
                    |r| {
                        Ok(Job {
                            id: r.get(0)?,
                            recording_id: r.get(1)?,
                            step: r.get(2)?,
                            attempts: r.get(3)?,
                        })
                    },
                )
                .optional()?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    async fn complete(&self, job: &Job, then: Option<&str>) -> Result<()> {
        let (id, recording_id) = (job.id, job.recording_id);
        let then = then.map(str::to_owned);
        let changed = self
            .run(move |c| {
                let tx = write_tx(c)?;
                let t = now();
                let changed = tx.execute(
                    "UPDATE jobs SET status = 'done', updated_at = ?2
                     WHERE id = ?1 AND status = 'running'",
                    params![id, t],
                )?;
                if changed != 1 {
                    return Ok(changed); // dropped transaction rolls back: nothing is enqueued
                }
                if let Some(step) = then {
                    tx.execute(
                        "INSERT INTO jobs (recording_id, step, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
                        params![recording_id, step, t],
                    )?;
                }
                tx.commit()?;
                Ok(changed)
            })
            .await?;
        expect_running(changed)
    }

    async fn fail(&self, job: &Job, error: &str) -> Result<()> {
        let (id, error) = (job.id, error.to_owned());
        let changed = self
            .run(move |c| {
                c.execute(
                    "UPDATE jobs SET status = 'failed', error = ?2, updated_at = ?3
                     WHERE id = ?1 AND status = 'running'",
                    params![id, error, now()],
                )
            })
            .await?;
        expect_running(changed)
    }

    /// Call once at startup, before claiming: it re-queues `running` jobs (failing those already at
    /// 3 attempts: a job that keeps killing the process must not loop forever) only when no other
    /// worker is alive. Liveness is a lock on `<db>.workers`: every worker holds it shared for
    /// the life of the process (taken here), so an exclusive lock succeeds only when all
    /// earlier workers are dead.
    async fn reset_stale(&self) -> Result<u64> {
        let sole = match self.workers.try_lock() {
            Ok(()) => true,
            Err(TryLockError::WouldBlock) => false,
            Err(TryLockError::Error(e)) => return Err(storage(e)),
        };
        let requeued = if sole {
            self.run(|c| {
                let t = now();
                c.execute(
                    "UPDATE jobs SET status = 'failed', error = 'stopped after repeated crashes', updated_at = ?1
                     WHERE status = 'running' AND attempts >= 3",
                    [&t],
                )?;
                let n = c.execute(
                    "UPDATE jobs SET status = 'queued', updated_at = ?1 WHERE status = 'running'",
                    [&t],
                )?;
                Ok(n as u64)
            })
            .await
        } else {
            Ok(0)
        };
        // Downgrade (or take) the shared lock and keep it, even if the requeue failed.
        let lock = Arc::clone(&self.workers);
        tokio::task::spawn_blocking(move || lock.lock_shared())
            .await
            .map_err(storage)?
            .map_err(storage)?;
        requeued
    }

    async fn list_for(&self, recording_id: i64) -> Result<Vec<JobInfo>> {
        self.run(move |c| {
            let mut stmt = c.prepare(
                "SELECT id, step, status, attempts, error, updated_at FROM jobs WHERE recording_id = ?1 ORDER BY id",
            )?;
            stmt.query_map([recording_id], |r| {
                Ok(JobInfo {
                    id: r.get(0)?,
                    step: r.get(1)?,
                    status: parse_status(&r.get::<_, String>(2)?)?,
                    attempts: r.get(3)?,
                    error: r.get(4)?,
                    updated_at: parse_ts(5, &r.get::<_, String>(5)?)?,
                })
            })?
            .collect()
        })
        .await
    }
}
