//! `liberarr process` end to end against an in-process mock provider, then `serve` on the result.
#![allow(clippy::unwrap_used)] // a test: a panic is the failure report
use axum::Router;
use axum::routing::post;
use liberarr_core::exports::{END, START};
use liberarr_core::jobs::JobQueue;
use liberarr_core::recordings::{RecordingRepo, RecordingStatus};
use liberarr_core::summaries::SummaryRepo;
use liberarr_core::transcription::TranscriptRepo;
use liberarr_infra::sqlite::Db;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const API_KEY: &str = "0123456789abcdef0123456789abcdef-api";

struct TempHome(PathBuf);
impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Bitwise Ogg CRC (poly 0x04C11DB7, init 0, no reflection).
fn crc(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |mut c, &b| {
        c ^= u32::from(b) << 24;
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            };
        }
        c
    })
}

fn page(seq: u32, flags: u8, granule: i64, body: &[u8]) -> Vec<u8> {
    let mut p = b"OggS\0".to_vec();
    p.push(flags);
    p.extend_from_slice(&granule.to_le_bytes());
    p.extend_from_slice(&7u32.to_le_bytes());
    p.extend_from_slice(&seq.to_le_bytes());
    p.extend_from_slice(&[0; 4]);
    p.extend_from_slice(&[1, body.len() as u8]);
    p.extend_from_slice(body);
    let c = crc(&p);
    p[22..26].copy_from_slice(&c.to_le_bytes());
    p
}

/// A tiny synthetic Ogg Opus stream: OpusHead, OpusTags, one audio page.
fn synthetic_ogg() -> Vec<u8> {
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 1, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0, 0, 0]);
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&4u32.to_le_bytes());
    tags.extend_from_slice(b"test");
    tags.extend_from_slice(&0u32.to_le_bytes());
    [
        page(0, 2, 0, &head),
        page(1, 0, 0, &tags),
        page(2, 4, 48_000, &[0xF8, 0xFF, 0xFE]),
    ]
    .concat()
}

const SUMMARY: &str = r###"{"title":"Point hebdo","summary_md":"## Contexte\nTest.","mindmap":"mindmap\n  root((Point))\n    Budget","actions":[{"action":"Envoyer le budget","owner":"Alice","due":null,"start":1.0}]}"###;

const SPEECH: &str = r#"{"language":"french","text":"Bonjour. Hello.","segments":[{"start":0.0,"end":1.0,"text":"Bonjour."},{"start":1.0,"end":2.0,"text":"Hello."}]}"#;

/// `transcript` is the transcription endpoint's reply; while `llm_ok` is off the LLM replies with
/// an invalid summary, so the summarize step fails.
async fn mock_provider(transcript: &'static str, llm_ok: Arc<AtomicBool>) -> String {
    let app = Router::new()
        .route(
            "/v1/audio/transcriptions",
            post(move || async move { transcript }),
        )
        .route(
            "/v1/chat/completions",
            post(move || async move {
                let content = if llm_ok.load(Ordering::SeqCst) {
                    SUMMARY
                } else {
                    "not a summary"
                };
                serde_json::json!({"choices":[{"message":{"role":"assistant","content":content}}]})
                    .to_string()
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/v1")
}

fn liberarr(home: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_liberarr"));
    c.env("LIBERARR_HOME", home);
    c
}

/// Status code of one raw HTTP/1.1 request.
fn http(port: u16, request: &str) -> std::io::Result<u16> {
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.write_all(request.as_bytes())?;
    let mut reply = String::new();
    s.read_to_string(&mut reply)?;
    Ok(reply
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0))
}

fn patch_action(port: u16, index: usize, action: &str) -> u16 {
    let body = format!(r#"{{"action":"{action}","done":true}}"#);
    http(
        port,
        &format!(
            "PATCH /api/recordings/1/actions/{index} HTTP/1.1\r\nHost: localhost:{port}\r\nX-Api-Key: {API_KEY}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
    .unwrap()
}

/// A fresh home (named after the test: tests run in parallel) whose `.env` points both steps at a
/// mock provider, plus a synthetic recording and the mock's `llm_ok` switch (on).
async fn setup(test: &str, transcript: &'static str) -> (TempHome, PathBuf, Arc<AtomicBool>) {
    let home = TempHome(std::env::temp_dir().join(format!(
        "liberarr-{test}-{}-{}",
        std::process::id(),
        jiff::Timestamp::now().as_nanosecond()
    )));
    std::fs::create_dir_all(&home.0).unwrap();
    let llm_ok = Arc::new(AtomicBool::new(true));
    let base = mock_provider(transcript, llm_ok.clone()).await;
    std::fs::write(
        home.0.join(".env"),
        format!(
            "LIBERARR_SECRET_KEY=fedcba9876543210fedcba9876543210-secret\n\
             LIBERARR_API_KEY={API_KEY}\n\
             LIBERARR_ADMIN_PASSWORD={pw}\n\
             LIBERARR_TRANSCRIBE_BASE_URL={base}\n\
             LIBERARR_TRANSCRIBE_API_KEY=sk-test\n\
             LIBERARR_TRANSCRIBE_MODEL=whisper-1\n\
             LIBERARR_LLM_BASE_URL={base}\n\
             LIBERARR_LLM_API_KEY=sk-test\n\
             LIBERARR_LLM_MODEL=gpt-test\n",
            pw = "pw".repeat(16),
        ),
    )
    .unwrap();
    let audio = home.0.join("meeting.ogg");
    std::fs::write(&audio, synthetic_ogg()).unwrap();
    (home, audio, llm_ok)
}

#[tokio::test(flavor = "multi_thread")]
async fn process_e2e() {
    let (home, audio, _) = setup("e2e", SPEECH).await;
    let out = liberarr(&home.0)
        .arg("process")
        .arg(&audio)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "process failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let db = Db::open(&home.0.join("liberarr.db")).unwrap();
    let items = db.list_items().await.unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, RecordingStatus::Done);
    let t = TranscriptRepo::get(&db, items[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.segments.len(), 2);
    let s = SummaryRepo::get(&db, items[0].id).await.unwrap().unwrap();
    assert_eq!(s.actions.len(), 1);
    let note = std::fs::read_dir(home.0.join("obsidian"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "md"))
        .expect("an Obsidian note");
    let note = std::fs::read_to_string(note).unwrap();
    assert!(note.contains(START) && note.contains(END) && note.contains("```mermaid"));
    // The original audio is left where it was.
    assert!(audio.exists());

    // `serve` on the same home: the action toggle goes through the real backend.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = liberarr(&home.0)
        .args(["serve", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let health =
        &format!("GET /api/health HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n");
    let up = (0..100).any(|_| {
        std::thread::sleep(Duration::from_millis(100));
        http(port, health).is_ok_and(|c| c == 200)
    });
    let stale = patch_action(port, 0, "Autre action");
    let (in_range, out_of_range) = (
        patch_action(port, 0, "Envoyer le budget"),
        patch_action(port, 5, "Envoyer le budget"),
    );
    server.kill().unwrap();
    let _ = server.wait();
    assert!(up, "serve did not come up");
    assert_eq!(stale, 409);
    assert_eq!(in_range, 200);
    assert_eq!(out_of_range, 404);
}

/// Silence: the empty transcript is kept, the LLM is not called, no note is written, and the
/// recording ends Done with the "no speech" marker.
#[tokio::test(flavor = "multi_thread")]
async fn process_no_speech() {
    let (home, audio, _) = setup(
        "no-speech",
        r#"{"language":null,"text":"  ","segments":[]}"#,
    )
    .await;
    let out = liberarr(&home.0)
        .arg("process")
        .arg(&audio)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "process failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let db = Db::open(&home.0.join("liberarr.db")).unwrap();
    let items = db.list_items().await.unwrap();
    assert_eq!(items[0].status, RecordingStatus::Done);
    assert!(items[0].no_speech);
    let id = items[0].id;
    assert_eq!(
        TranscriptRepo::get(&db, id).await.unwrap().unwrap().text,
        ""
    );
    assert!(SummaryRepo::get(&db, id).await.unwrap().is_none());
    let steps: Vec<String> = db
        .list_for(id)
        .await
        .unwrap()
        .into_iter()
        .map(|j| j.step)
        .collect();
    assert_eq!(
        steps,
        ["transcribe", "summarize"],
        "the chain stops before the export"
    );
    let notes = std::fs::read_dir(home.0.join("obsidian")).map_or(0, |d| d.count());
    assert_eq!(notes, 0);
}

/// A failed summary keeps the transcript; `rerun` then the next `process` reach Done. A rerun of
/// the export rewrites the same note and keeps the owner's text outside the markers.
#[tokio::test(flavor = "multi_thread")]
async fn rerun_after_failure_keeps_outputs_and_owner_text() {
    let (home, audio, llm_ok) = setup("rerun", SPEECH).await;
    let process = || {
        liberarr(&home.0)
            .arg("process")
            .arg(&audio)
            .output()
            .unwrap()
    };
    let rerun = |args: &[&str]| {
        let out = liberarr(&home.0).arg("rerun").args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    llm_ok.store(false, Ordering::SeqCst);
    assert!(!process().status.success());

    let db = Db::open(&home.0.join("liberarr.db")).unwrap();
    let items = db.list_items().await.unwrap();
    assert_eq!(items[0].status, RecordingStatus::Failed);
    let id = items[0].id.to_string();
    let transcript = TranscriptRepo::get(&db, items[0].id)
        .await
        .unwrap()
        .unwrap();

    llm_ok.store(true, Ordering::SeqCst);
    rerun(&[&id]);
    let out = process();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        db.list_items().await.unwrap()[0].status,
        RecordingStatus::Done
    );
    let kept = TranscriptRepo::get(&db, items[0].id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kept.text, transcript.text);
    assert_eq!(kept.segments.len(), transcript.segments.len());
    let jobs = db.list_for(items[0].id).await.unwrap();
    let transcribes = jobs.iter().filter(|j| j.step == "transcribe").count();
    assert_eq!(transcribes, 1, "the rerun starts at the failed step");
    assert!(audio.exists());

    let vault = home.0.join("obsidian");
    let note = std::fs::read_dir(&vault)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let edited = format!(
        "my intro\n{}my own notes\n",
        std::fs::read_to_string(&note).unwrap()
    );
    std::fs::write(&note, edited).unwrap();
    rerun(&[&id, "--step", "export"]);
    assert!(process().status.success());
    assert_eq!(
        std::fs::read_dir(&vault).unwrap().count(),
        1,
        "same note rewritten"
    );
    let after = std::fs::read_to_string(&note).unwrap();
    assert!(
        after.starts_with("my intro\n") && after.ends_with("my own notes\n"),
        "{after}"
    );
    assert_eq!(after.matches(START).count(), 1);
}
