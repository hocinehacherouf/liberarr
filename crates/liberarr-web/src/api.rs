//! The JSON API. Handlers are thin: they call the `Backend` and shape the contract.
use crate::{AppState, Backend};
use axum::Json;
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::uri::Authority;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use liberarr_core::Error;
use liberarr_core::diarization::{SpeakerTurn, speaker_for};
use liberarr_core::jobs::{JobInfo, JobStatus, STEPS, busy, export_needed};
use liberarr_core::recordings::{RecordingItem, RecordingStatus};
use liberarr_core::search::query_ok;
use liberarr_core::summaries::{Action, MeetingSummary, SummaryText};
use liberarr_core::transcription::{
    SegmentChange, SegmentEdit, Transcript, apply_edits, unit_texts,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// `{"detail": "..."}`. Details are fixed phrases: never transcript text, paths or secrets.
pub struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "detail": self.1 }))).into_response()
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        match e {
            Error::NotFound(_) => ApiError(StatusCode::NOT_FOUND, "not found"),
            Error::Conflict(_) => ApiError(StatusCode::CONFLICT, "conflict"),
            _ => ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
        }
    }
}

const NOT_FOUND: ApiError = ApiError(StatusCode::NOT_FOUND, "not found");
const INVALID_EDIT: ApiError = ApiError(StatusCode::BAD_REQUEST, "invalid edit");
const UNAUTHORIZED: ApiError = ApiError(StatusCode::UNAUTHORIZED, "unauthorized");

pub async fn not_found() -> ApiError {
    NOT_FOUND
}

pub async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok" }))
}

// --- auth ---

pub async fn require_auth<B: Backend>(
    State(s): State<AppState<B>>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let ok = {
        let h = req.headers();
        let get = |name: &str| h.get(name).and_then(|v| v.to_str().ok());
        s.auth.authorized(get("x-api-key"), get("cookie"))
    };
    if ok {
        Ok(next.run(req).await)
    } else {
        Err(UNAUTHORIZED)
    }
}

#[derive(Deserialize)]
pub struct Login {
    username: String,
    password: String,
}

/// Needs `into_make_service_with_connect_info::<SocketAddr>()` on the server.
pub async fn login<B: Backend>(
    State(s): State<AppState<B>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(body): Json<Login>,
) -> Result<Response, ApiError> {
    let ip = peer.ip();
    if !s.auth.begin_attempt(ip) {
        return Err(ApiError(StatusCode::TOO_MANY_REQUESTS, "too many attempts"));
    }
    if !s.auth.credentials_ok(&body.username, &body.password) {
        return Err(UNAUTHORIZED);
    }
    s.auth.clear_failures(ip);
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, s.auth.session_cookie())],
    )
        .into_response())
}

pub async fn logout<B: Backend>(State(s): State<AppState<B>>) -> Response {
    (
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, s.auth.logout())],
    )
        .into_response()
}

pub async fn me<B: Backend>(State(s): State<AppState<B>>) -> Json<serde_json::Value> {
    Json(json!({ "username": s.auth.username() }))
}

// --- recordings ---

#[derive(Serialize)]
pub struct ItemView {
    id: i64,
    title: Option<String>,
    source: &'static str,
    duration_s: Option<f64>,
    created_at: String,
    status: &'static str,
    failed_step: Option<String>,
    no_speech: bool,
}

impl From<RecordingItem> for ItemView {
    fn from(i: RecordingItem) -> Self {
        Self {
            id: i.id,
            title: i.title,
            source: i.source.as_str(),
            duration_s: i.duration_s,
            created_at: i.created_at.to_string(),
            status: match i.status {
                RecordingStatus::Processing => "processing",
                RecordingStatus::Failed => "failed",
                RecordingStatus::Done => "done",
            },
            failed_step: i.failed_step,
            no_speech: i.no_speech,
        }
    }
}

#[derive(Serialize)]
pub struct SegmentView {
    start: f64,
    end: f64,
    /// The corrected text, `""` when removed: what a client sends back as `current`.
    text: String,
    /// The engine's text, when an edit applies to this segment.
    original: Option<String>,
    removed: bool,
    /// 0-based; `None` without a diarization or where no speaker covers the segment.
    speaker: Option<u32>,
}

#[derive(Serialize)]
pub struct TranscriptView {
    engine: String,
    language: Option<String>,
    text: String,
    /// The engine's text, for a transcript without timestamps whose one unit is edited.
    text_original: Option<String>,
    has_timestamps: bool,
    /// Empty without timestamps: the whole text is the one editable unit.
    segments: Vec<SegmentView>,
}

impl TranscriptView {
    fn new(raw: Transcript, edits: &[SegmentEdit], turns: &[SpeakerTurn]) -> Self {
        let shown = unit_texts(&raw, edits);
        // A stored edit never equals its raw unit (a revert deletes it), so a differing text is an edit.
        let (segments, text_original) = if raw.has_timestamps {
            let segments = raw
                .segments
                .iter()
                .zip(&shown)
                .map(|(s, &text)| {
                    let edited = text != s.text;
                    SegmentView {
                        speaker: speaker_for(s.start, s.end, turns),
                        start: s.start,
                        end: s.end,
                        text: text.into(),
                        original: edited.then(|| s.text.clone()),
                        removed: edited && text.is_empty(),
                    }
                })
                .collect();
            (segments, None)
        } else {
            (Vec::new(), (shown[0] != raw.text).then(|| raw.text.clone()))
        };
        let t = apply_edits(raw, edits);
        Self {
            engine: t.engine,
            language: t.language,
            text: t.text,
            text_original,
            has_timestamps: t.has_timestamps,
            segments,
        }
    }
}

#[derive(Serialize)]
pub struct JobView {
    id: i64,
    step: String,
    status: &'static str,
    attempts: u32,
    error: Option<String>,
    updated_at: String,
}

impl From<JobInfo> for JobView {
    fn from(j: JobInfo) -> Self {
        Self {
            id: j.id,
            step: j.step,
            status: match j.status {
                JobStatus::Queued => "queued",
                JobStatus::Running => "running",
                JobStatus::Failed => "failed",
                JobStatus::Done => "done",
            },
            attempts: j.attempts,
            error: j.error,
            updated_at: j.updated_at.to_string(),
        }
    }
}

#[derive(Serialize)]
pub struct SpeakersView {
    requested: Option<u32>,
    /// By 0-based speaker (JSON keys are strings).
    names: BTreeMap<u32, String>,
}

#[derive(Serialize)]
pub struct GeneratedView {
    title: String,
    summary_md: String,
}

/// The summary as every reader sees it (the owner's title and Markdown when edited), plus what was generated.
#[derive(Serialize)]
pub struct SummaryView {
    #[serde(flatten)]
    summary: MeetingSummary,
    generated: Option<GeneratedView>,
}

impl SummaryView {
    fn new(generated: MeetingSummary, edit: Option<SummaryText>) -> Self {
        let Some(edit) = edit else {
            return Self {
                summary: generated,
                generated: None,
            };
        };
        Self {
            generated: Some(GeneratedView {
                title: generated.title.clone(),
                summary_md: generated.summary_md.clone(),
            }),
            summary: MeetingSummary {
                title: edit.title,
                summary_md: edit.summary_md,
                ..generated
            },
        }
    }
}

#[derive(Serialize)]
pub struct Detail {
    #[serde(flatten)]
    item: ItemView,
    transcript: Option<TranscriptView>,
    speakers: SpeakersView,
    summary: Option<SummaryView>,
    jobs: Vec<JobView>,
}

pub async fn list<B: Backend>(
    State(s): State<AppState<B>>,
) -> Result<Json<Vec<ItemView>>, ApiError> {
    let items = s.backend.list().await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
}

const SEARCH_HITS: usize = 20;

#[derive(Deserialize)]
pub struct Search {
    q: String,
}

#[derive(Serialize)]
pub struct HitView {
    recording_id: i64,
    start: Option<f64>,
    before: String,
    #[serde(rename = "match")]
    hit: String,
    after: String,
}

#[derive(Serialize)]
pub struct SearchView {
    hits: Vec<HitView>,
    truncated: bool,
}

/// The query and the matches are never logged, and the errors are fixed phrases.
pub async fn search<B: Backend>(
    State(s): State<AppState<B>>,
    Json(body): Json<Search>,
) -> Result<Json<SearchView>, ApiError> {
    if !query_ok(&body.q) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid query"));
    }
    // One more than shown tells whether the list was cut.
    let hits = s.backend.search(body.q.trim(), SEARCH_HITS + 1).await?;
    let truncated = hits.len() > SEARCH_HITS;
    let hits = hits
        .into_iter()
        .take(SEARCH_HITS)
        .map(|h| HitView {
            recording_id: h.recording_id,
            start: h.start,
            before: h.excerpt.before,
            hit: h.excerpt.hit,
            after: h.excerpt.after,
        })
        .collect();
    Ok(Json(SearchView { hits, truncated }))
}

pub async fn detail<B: Backend>(
    State(s): State<AppState<B>>,
    Path(id): Path<i64>,
) -> Result<Json<Detail>, ApiError> {
    // The list carries the derived status, so the item is found there rather than re-derived.
    let item = s
        .backend
        .list()
        .await?
        .into_iter()
        .find(|i| i.id == id)
        .ok_or(NOT_FOUND)?;
    let diarization = s.backend.diarization(id).await?;
    let turns = diarization.as_ref().map_or(&[][..], |d| &d.turns[..]);
    Ok(Json(Detail {
        item: item.into(),
        transcript: s
            .backend
            .transcript_with_edits(id)
            .await?
            .map(|(t, edits)| TranscriptView::new(t, &edits, turns)),
        speakers: SpeakersView {
            requested: s.backend.requested_speakers(id).await?,
            names: s.backend.speaker_names(id).await?,
        },
        summary: s
            .backend
            .summary_with_edit(id)
            .await?
            .map(|(g, edit)| SummaryView::new(g, edit)),
        jobs: s
            .backend
            .jobs(id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}

/// The path comes from the repo, never from the request. `ServeFile` handles Range.
pub async fn audio<B: Backend>(
    State(s): State<AppState<B>>,
    Path(id): Path<i64>,
    req: Request,
) -> Result<Response, ApiError> {
    let rec = s.backend.recording(id).await?.ok_or(NOT_FOUND)?;
    let Ok(res) = ServeFile::new(rec.audio_path).oneshot(req).await;
    Ok(res.into_response())
}

#[derive(Deserialize)]
pub struct Rerun {
    step: String,
}

pub async fn rerun<B: Backend>(
    State(s): State<AppState<B>>,
    Path(id): Path<i64>,
    Json(body): Json<Rerun>,
) -> Result<StatusCode, ApiError> {
    let step = STEPS
        .iter()
        .find(|st| **st == body.step)
        .ok_or(ApiError(StatusCode::BAD_REQUEST, "unknown step"))?;
    s.backend.recording(id).await?.ok_or(NOT_FOUND)?;
    // A double click, or a rerun while the pipeline runs, must not run the chain (AI calls) twice.
    // shortcut: check-then-enqueue is not atomic; two simultaneous requests could both pass.
    if !busy(&s.backend.jobs(id).await?) {
        s.backend.enqueue(id, step).await?;
    }
    Ok(StatusCode::ACCEPTED)
}

const MAX_SPEAKER_NAME: usize = 60;
const MAX_SPEAKERS: u32 = 20;

#[derive(Deserialize)]
pub struct SpeakerName {
    name: String,
}

/// Blank resets the name. The note follows when nothing else is running (an export in the queue
/// will already read the new name).
pub async fn rename_speaker<B: Backend>(
    State(s): State<AppState<B>>,
    Path((id, n)): Path<(i64, u32)>,
    Json(body): Json<SpeakerName>,
) -> Result<StatusCode, ApiError> {
    let name = body.name.trim();
    if name.chars().count() > MAX_SPEAKER_NAME || name.chars().any(char::is_control) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid name"));
    }
    s.backend.recording(id).await?.ok_or(NOT_FOUND)?;
    let known = s
        .backend
        .diarization(id)
        .await?
        .is_some_and(|d| d.turns.iter().any(|t| t.speaker == n));
    if !known {
        return Err(NOT_FOUND);
    }
    s.backend
        .set_speaker_name(id, n, Some(name).filter(|n| !n.is_empty()))
        .await?;
    export_after_edit(&s, id).await
}

#[derive(Deserialize)]
pub struct SpeakerCount {
    count: Option<u32>,
}

/// Saves the count (`null` clears it) and diarizes again. Refused while the recording is busy
/// (the running diarization would not see it) and when diarization is off.
pub async fn speaker_count<B: Backend>(
    State(s): State<AppState<B>>,
    Path(id): Path<i64>,
    Json(body): Json<SpeakerCount>,
) -> Result<StatusCode, ApiError> {
    if body.count.is_some_and(|c| !(1..=MAX_SPEAKERS).contains(&c)) {
        return Err(ApiError(StatusCode::BAD_REQUEST, "invalid speaker count"));
    }
    s.backend.recording(id).await?.ok_or(NOT_FOUND)?;
    if busy(&s.backend.jobs(id).await?) {
        return Err(ApiError(StatusCode::CONFLICT, "conflict"));
    }
    s.backend.set_speaker_count(id, body.count).await?;
    s.backend.enqueue(id, "diarize").await?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize)]
pub struct Toggle {
    /// The text the client saw at that index: the server refuses with 409 when it has changed.
    action: String,
    done: bool,
}

pub async fn toggle_action<B: Backend>(
    State(s): State<AppState<B>>,
    Path((id, index)): Path<(i64, usize)>,
    Json(body): Json<Toggle>,
) -> Result<Json<Vec<Action>>, ApiError> {
    Ok(Json(
        s.backend
            .toggle_action(id, index, &body.action, body.done)
            .await?
            .actions,
    ))
}

// axum's default 2 MB body limit bounds the whole payload, whatever these limits add up to.
const MAX_EDITS: usize = 5000;
const MAX_EDIT_CHARS: usize = 500_000;
const MAX_TITLE: usize = 200;
const MAX_SUMMARY_CHARS: usize = 100_000;

/// A transcription (or summary) queued or running will replace what is being edited.
// shortcut: the check and the write are not atomic; a transcription that starts in between makes the edit stale, and a
// stale edit is ignored on read. Fine for a single owner.
fn replacing(jobs: &[JobInfo], steps: &[&str]) -> bool {
    jobs.iter().any(|j| {
        matches!(j.status, JobStatus::Queued | JobStatus::Running)
            && steps.contains(&j.step.as_str())
    })
}

/// The edit is saved: the note follows unless a queued or running chain will export it.
async fn export_after_edit<B: Backend>(s: &AppState<B>, id: i64) -> Result<StatusCode, ApiError> {
    // shortcut: check-then-enqueue is not atomic (same as `rerun`).
    if export_needed(&s.backend.jobs(id).await?) {
        s.backend.enqueue(id, "export").await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct TranscriptEdits {
    edits: Vec<EditIn>,
}

#[derive(Deserialize)]
struct EditIn {
    index: usize,
    /// The text the client displayed for that unit: the server refuses with 409 when it has changed.
    current: String,
    text: String,
}

pub async fn edit_transcript<B: Backend>(
    State(s): State<AppState<B>>,
    Path(id): Path<i64>,
    Json(body): Json<TranscriptEdits>,
) -> Result<StatusCode, ApiError> {
    let edits = body.edits;
    let mut seen = HashSet::new();
    // shortcut: reverting a raw text that holds another control character is refused; Whisper output has none
    let valid = (1..=MAX_EDITS).contains(&edits.len())
        && edits.iter().all(|e| {
            seen.insert(e.index)
                && e.text.chars().count() <= MAX_EDIT_CHARS
                && e.current.chars().count() <= MAX_EDIT_CHARS
                && !e
                    .text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
        });
    if !valid {
        return Err(INVALID_EDIT);
    }
    s.backend.recording(id).await?.ok_or(NOT_FOUND)?;
    if replacing(&s.backend.jobs(id).await?, &["transcribe"]) {
        return Err(ApiError(StatusCode::CONFLICT, "conflict"));
    }
    let changes: Vec<SegmentChange> = edits
        .into_iter()
        .map(|e| SegmentChange {
            index: e.index,
            current: e.current,
            text: e.text,
        })
        .collect();
    s.backend.set_transcript_edits(id, &changes).await?;
    export_after_edit(&s, id).await
}

#[derive(Deserialize)]
pub struct SummaryEdit {
    title: String,
    summary_md: String,
    generated_title: String,
    generated_summary_md: String,
    /// What the page displayed (the owner's version, else the generated one): 409 when it has changed since.
    current_title: String,
    current_summary_md: String,
}

pub async fn edit_summary<B: Backend>(
    State(s): State<AppState<B>>,
    Path(id): Path<i64>,
    Json(body): Json<SummaryEdit>,
) -> Result<StatusCode, ApiError> {
    let SummaryEdit {
        title,
        summary_md,
        generated_title,
        generated_summary_md,
        current_title,
        current_summary_md,
    } = body;
    let too_long = [
        &title,
        &summary_md,
        &generated_title,
        &generated_summary_md,
        &current_title,
        &current_summary_md,
    ]
    .iter()
    .any(|f| f.chars().count() > MAX_EDIT_CHARS);
    // A field equal to its generated text passes as sent, so a restore compares equal and drops the overlay.
    let title_ok = title == generated_title || {
        let t = title.trim();
        (1..=MAX_TITLE).contains(&t.chars().count()) && !t.chars().any(char::is_control)
    };
    let md_ok = summary_md == generated_summary_md
        || (summary_md.chars().count() <= MAX_SUMMARY_CHARS
            && !summary_md
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')));
    if too_long || !title_ok || !md_ok {
        return Err(INVALID_EDIT);
    }
    s.backend.recording(id).await?.ok_or(NOT_FOUND)?;
    if replacing(&s.backend.jobs(id).await?, &["transcribe", "summarize"]) {
        return Err(ApiError(StatusCode::CONFLICT, "conflict"));
    }
    let title = if title == generated_title {
        title
    } else {
        title.trim().to_owned()
    };
    s.backend
        .set_summary_edit(
            id,
            &SummaryText {
                title: generated_title,
                summary_md: generated_summary_md,
            },
            &SummaryText {
                title: current_title,
                summary_md: current_summary_md,
            },
            &SummaryText { title, summary_md },
        )
        .await?;
    export_after_edit(&s, id).await
}

/// Inline styles stay allowed: mermaid puts a `<style>` element inside the mind map SVG.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
    object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

pub async fn security_headers(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(
        "x-content-type-options",
        "nosniff".parse().expect("static value"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        CSP.parse().expect("static value"),
    );
    res
}

/// On a loopback bind, only `127.0.0.1`, `localhost` or `[::1]` on the bound port may be named
/// in `Host`: a DNS-rebinding page cannot reach the API (or burn the login rate limit).
pub async fn loopback_host_only(State(port): State<u16>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.parse::<Authority>().ok());
    let ok = host.is_some_and(|a| {
        matches!(a.host(), "127.0.0.1" | "localhost" | "[::1]")
            && a.port_u16().unwrap_or(80) == port
    });
    if ok {
        next.run(req).await
    } else {
        StatusCode::MISDIRECTED_REQUEST.into_response()
    }
}

pub async fn no_store(mut res: Response) -> Response {
    res.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static value"),
    );
    res
}
