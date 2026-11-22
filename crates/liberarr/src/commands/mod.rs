//! One module per CLI command. Output is for the owner, translated (`locales/`); it never shows a token, a key,
//! a full serial, a transcript or a summary (`mcp` excepted: it serves them, read-only, to the
//! owner's AI agent on stdout, which carries protocol messages only).
pub mod backup;
pub mod init;
pub mod link;
pub mod mcp;
pub mod process;
pub mod pull;
pub mod rerun;
pub mod serve;
pub mod status;

use crate::container::App;
use liberarr_core::jobs::{JobQueue, JobStatus, OPTIONAL_STEPS, latest_by_step};
use liberarr_core::transcription::TranscriptRepo;
use rust_i18n::t;
use std::io::{BufRead, IsTerminal, Write};

pub type Outcome = Result<std::process::ExitCode, Box<dyn std::error::Error>>;

/// One line when the notes' language is not pinned in the `.env`: it would follow the OS locale.
pub fn note_content_language(cfg: &crate::config::Config) {
    if !cfg.content_language_set {
        eprintln!(
            "{}",
            t!(
                "config.content_language_follows_ui",
                language = cfg.content_language.as_str()
            )
        );
    }
}

/// One line before a worker starts (`serve`, `process`, `pull`) when the first run will download a local model.
/// Not for the TUI, which shares the worker and owns the screen.
pub fn note_model_download(app: &App) {
    if let Some(notice) = app.model_download_notice() {
        eprintln!("{notice}");
    }
}

/// Reads one line. `hidden` input is not echoed on a terminal; piped input is read as is.
pub fn ask(prompt: &str, hidden: bool) -> std::io::Result<String> {
    let line = if hidden && std::io::stdin().is_terminal() {
        rpassword::prompt_password(prompt)?
    } else {
        eprint!("{prompt}");
        std::io::stderr().flush()?;
        let mut line = String::new();
        if std::io::stdin().lock().read_line(&mut line)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                t!("ask.input_closed").into_owned(),
            ));
        }
        line
    };
    Ok(line.trim().to_string())
}

/// Runs the queue to the end (also work left by an earlier `pull --no-process`), then prints
/// where each of `ids` stands. `false` if a required step failed; a failed optional step is a warning.
pub async fn process_and_report(app: &App, ids: &[i64]) -> Result<bool, liberarr_core::Error> {
    note_model_download(app);
    eprintln!("{}", t!("process.running"));
    app.drain().await?;
    if ids.is_empty() {
        return Ok(true);
    }
    let mut ok = true;
    for &id in ids {
        let jobs = app.db.list_for(id).await?;
        // The latest job of each step: a failed optional step shows although the export ran after it.
        let mut required_failed = false;
        for j in latest_by_step(&jobs)
            .into_iter()
            .filter(|j| j.status == JobStatus::Failed)
        {
            let optional = OPTIONAL_STEPS.contains(&j.step.as_str());
            required_failed |= !optional;
            let key = if optional {
                "process.optional_failed"
            } else {
                "process.failed_step"
            };
            println!(
                "{}",
                t!(
                    key,
                    id = id,
                    step = j.step,
                    error = j.error.as_deref().unwrap_or("?")
                )
            );
        }
        ok &= !required_failed;
        match jobs.last() {
            _ if required_failed => {}
            Some(j) if j.status == JobStatus::Done => {
                let silent = TranscriptRepo::get(&app.db, id)
                    .await?
                    .is_some_and(|t| !t.has_speech());
                let key = if silent {
                    "process.done_no_speech"
                } else {
                    "process.done"
                };
                println!("{}", t!(key, id = id));
            }
            _ => println!("{}", t!("process.waiting", id = id)),
        }
    }
    if ok {
        println!(
            "{}",
            t!("process.notes_dir", path = app.cfg.obsidian.display())
        );
    } else {
        println!("{}", t!("process.retry_hint"));
    }
    Ok(ok)
}
