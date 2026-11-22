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
use liberarr_core::jobs::{JobInfo, JobStatus, STEPS, busy};
use liberarr_core::recordings::{RecordingItem, RecordingStatus};
use liberarr_core::summaries::{Action, MeetingSummary};
use liberarr_core::transcription::Transcript;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
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
    text: String,
    /// 0-based; `None` without a diarization or where no speaker covers the segment.
    speaker: Option<u32>,
}

#[derive(Serialize)]
pub struct TranscriptView {
    engine: String,
    language: Option<String>,
    text: String,
    has_timestamps: bool,
    segments: Vec<SegmentView>,
}

impl TranscriptView {
    fn new(t: Transcript, turns: &[SpeakerTurn]) -> Self {
        Self {
            engine: t.engine,
            language: t.language,
            text: t.text,
            has_timestamps: t.has_timestamps,
            segments: t
                .segments
                .into_iter()
                .map(|s| SegmentView {
                    speaker: speaker_for(s.start, s.end, turns),
                    start: s.start,
                    end: s.end,
                    text: s.text,
                })
                .collect(),
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
pub struct Detail {
    #[serde(flatten)]
    item: ItemView,
    transcript: Option<TranscriptView>,
    speakers: SpeakersView,
    summary: Option<MeetingSummary>,
    jobs: Vec<JobView>,
}

pub async fn list<B: Backend>(
    State(s): State<AppState<B>>,
) -> Result<Json<Vec<ItemView>>, ApiError> {
    let items = s.backend.list().await?;
    Ok(Json(items.into_iter().map(Into::into).collect()))
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
            .transcript(id)
            .await?
            .map(|t| TranscriptView::new(t, turns)),
        speakers: SpeakersView {
            requested: s.backend.requested_speakers(id).await?,
            names: s.backend.speaker_names(id).await?,
        },
        summary: s.backend.summary(id).await?,
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
    // shortcut: check-then-enqueue is not atomic (same as `rerun`).
    if !busy(&s.backend.jobs(id).await?) {
        s.backend.enqueue(id, "export").await?;
    }
    Ok(StatusCode::NO_CONTENT)
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
