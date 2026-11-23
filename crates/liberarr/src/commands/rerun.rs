//! `liberarr rerun <id> [--step]`: queues a step again (the failed one by default), never twice.
use super::Outcome;
use crate::container::App;
use liberarr_core::jobs::{Rerun, rerun};
use liberarr_core::recordings::RecordingRepo;
use liberarr_core::summaries::SummaryEditRepo;
use liberarr_core::transcription::TranscriptEditRepo;
use rust_i18n::t;
use std::process::ExitCode;

/// What a rerun could drop, read **before** the step is queued (a worker may run it and drop the edits right after).
pub struct EditsAtRisk {
    pub transcript: bool,
    pub summary: bool,
}

pub async fn edits_at_risk(app: &App, id: i64) -> liberarr_core::Result<EditsAtRisk> {
    let transcript = TranscriptEditRepo::get_with_edits(&app.db, id)
        .await?
        .is_some_and(|(_, edits)| !edits.is_empty());
    let summary = SummaryEditRepo::get_with_edit(&app.db, id)
        .await?
        .is_some_and(|(_, edit)| edit.is_some());
    Ok(EditsAtRisk {
        transcript,
        summary,
    })
}

impl EditsAtRisk {
    /// `"transcribe"`: the transcript line when `transcript`, then the summary line when `summary`;
    /// `"summarize"`: the summary line only; any other step: empty.
    pub fn warnings(&self, step: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if step == "transcribe" && self.transcript {
            lines.push(t!("rerun.drops_transcript_edits").into_owned());
        }
        if matches!(step, "transcribe" | "summarize") && self.summary {
            lines.push(t!("rerun.drops_summary_edit").into_owned());
        }
        lines
    }
}

pub async fn run(app: &App, id: i64, step: Option<&str>) -> Outcome {
    if RecordingRepo::get(&app.db, id).await?.is_none() {
        eprintln!("{}", t!("rerun.unknown_id", id = id));
        return Ok(ExitCode::FAILURE);
    }
    let at_risk = edits_at_risk(app, id).await?;
    match rerun(&app.db, id, step).await? {
        Rerun::Queued(step) => {
            println!("{}", t!("rerun.queued", id = id, step = step));
            for line in at_risk.warnings(&step) {
                eprintln!("{line}");
            }
            Ok(ExitCode::SUCCESS)
        }
        Rerun::Busy => {
            eprintln!("{}", t!("rerun.busy", id = id));
            Ok(ExitCode::FAILURE)
        }
        Rerun::NothingFailed => {
            eprintln!("{}", t!("rerun.nothing_failed", id = id));
            Ok(ExitCode::FAILURE)
        }
    }
}
