//! HTTP API and the embedded web UI.
//!
//! The core ports are native `async fn` traits whose futures are not provably `Send`, which axum
//! requires. `Backend` is the one seam: the same operations with `Send` futures, implemented by
//! the composition root where the concrete (SQLite) types make the bound provable.
mod api;
pub mod assets;
pub mod auth;

use axum::Router;
use axum::middleware::{from_fn_with_state, map_response};
use axum::routing::{get, patch, post, put};
use liberarr_core::Result;
use liberarr_core::diarization::Diarization;
use liberarr_core::jobs::JobInfo;
use liberarr_core::recordings::{Recording, RecordingItem};
use liberarr_core::summaries::{MeetingSummary, SummaryText};
use liberarr_core::transcription::{SegmentChange, SegmentEdit, Transcript};
use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

pub use auth::{Auth, AuthConfig};

pub trait Backend: Send + Sync + 'static {
    fn list(&self) -> impl Future<Output = Result<Vec<RecordingItem>>> + Send;
    fn recording(&self, id: i64) -> impl Future<Output = Result<Option<Recording>>> + Send;
    fn transcript(&self, id: i64) -> impl Future<Output = Result<Option<Transcript>>> + Send;
    fn summary(&self, id: i64) -> impl Future<Output = Result<Option<MeetingSummary>>> + Send;
    /// The engine's transcript, untouched, with the owner's edits.
    fn transcript_with_edits(
        &self,
        id: i64,
    ) -> impl Future<Output = Result<Option<(Transcript, Vec<SegmentEdit>)>>> + Send;
    /// The generated summary, untouched, and the owner's title and summary when edited.
    fn summary_with_edit(
        &self,
        id: i64,
    ) -> impl Future<Output = Result<Option<(MeetingSummary, Option<SummaryText>)>>> + Send;
    /// `Error::NotFound` / `Error::Conflict` as `TranscriptEditRepo::set_edits`.
    fn set_transcript_edits(
        &self,
        id: i64,
        changes: &[SegmentChange],
    ) -> impl Future<Output = Result<()>> + Send;
    /// `Error::NotFound` / `Error::Conflict` as `SummaryEditRepo::set_edit`.
    fn set_summary_edit(
        &self,
        id: i64,
        generated: &SummaryText,
        edit: &SummaryText,
    ) -> impl Future<Output = Result<()>> + Send;
    fn jobs(&self, id: i64) -> impl Future<Output = Result<Vec<JobInfo>>> + Send;
    /// Queues `step` (already validated against `STEPS`); the worker picks it up.
    fn enqueue(&self, id: i64, step: &str) -> impl Future<Output = Result<i64>> + Send;
    fn diarization(&self, id: i64) -> impl Future<Output = Result<Option<Diarization>>> + Send;
    /// The names the owner gave, by 0-based speaker.
    fn speaker_names(&self, id: i64) -> impl Future<Output = Result<BTreeMap<u32, String>>> + Send;
    /// The speaker count the owner set, if any.
    fn requested_speakers(&self, id: i64) -> impl Future<Output = Result<Option<u32>>> + Send;
    /// `None` resets the speaker to its default label.
    fn set_speaker_name(
        &self,
        id: i64,
        speaker: u32,
        name: Option<&str>,
    ) -> impl Future<Output = Result<()>> + Send;
    /// `Error::Conflict` when diarization is turned off.
    fn set_speaker_count(
        &self,
        id: i64,
        count: Option<u32>,
    ) -> impl Future<Output = Result<()>> + Send;
    /// `Error::NotFound` when the summary or the action index does not exist,
    /// `Error::Conflict` when the action at `index` no longer reads `action`.
    fn toggle_action(
        &self,
        id: i64,
        index: usize,
        action: &str,
        done: bool,
    ) -> impl Future<Output = Result<MeetingSummary>> + Send;
}

pub struct AppState<B> {
    pub backend: Arc<B>,
    pub auth: Arc<Auth>,
}

impl<B> AppState<B> {
    pub fn new(backend: B, auth: AuthConfig) -> Self {
        Self {
            backend: Arc::new(backend),
            auth: Arc::new(Auth::new(auth)),
        }
    }
}

// Manual: `derive(Clone)` would demand `B: Clone`.
impl<B> Clone for AppState<B> {
    fn clone(&self) -> Self {
        Self {
            backend: Arc::clone(&self.backend),
            auth: Arc::clone(&self.auth),
        }
    }
}

/// Serve it with `into_make_service_with_connect_info::<SocketAddr>()` (login rate limit).
/// `bind` is the listening address: on loopback, foreign `Host` headers are refused.
pub fn router<B: Backend>(state: AppState<B>, bind: SocketAddr) -> Router {
    let open = Router::new()
        .route("/health", get(api::health))
        .route("/auth/login", post(api::login::<B>));
    let protected = Router::new()
        .route("/auth/logout", post(api::logout::<B>))
        .route("/auth/me", get(api::me::<B>))
        .route("/recordings", get(api::list::<B>))
        .route("/recordings/{id}", get(api::detail::<B>))
        .route("/recordings/{id}/audio", get(api::audio::<B>))
        .route("/recordings/{id}/rerun", post(api::rerun::<B>))
        .route(
            "/recordings/{id}/speakers/{n}",
            put(api::rename_speaker::<B>),
        )
        .route(
            "/recordings/{id}/speaker-count",
            put(api::speaker_count::<B>),
        )
        .route(
            "/recordings/{id}/actions/{index}",
            patch(api::toggle_action::<B>),
        )
        .route(
            "/recordings/{id}/transcript",
            patch(api::edit_transcript::<B>),
        )
        .route("/recordings/{id}/summary", put(api::edit_summary::<B>))
        .layer(from_fn_with_state(state.clone(), api::require_auth::<B>));
    let app = Router::new()
        .nest(
            "/api",
            open.merge(protected)
                .fallback(api::not_found)
                .layer(map_response(api::no_store)),
        )
        .fallback(get(assets::spa))
        .layer(map_response(api::security_headers))
        .with_state(state);
    if bind.ip().is_loopback() {
        app.layer(from_fn_with_state(bind.port(), api::loopback_host_only))
    } else {
        app
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode, header};
    use liberarr_core::diarization::SpeakerTurn;
    use liberarr_core::jobs::JobStatus;
    use liberarr_core::recordings::{RecordingStatus, Source};
    use liberarr_core::summaries::Action;
    use liberarr_core::transcription::Segment;
    use std::sync::Mutex;
    use tower::ServiceExt;

    /// One recording (id 1) whose summary has a single action.
    #[derive(Default)]
    struct Fake {
        jobs: Arc<Mutex<Vec<JobInfo>>>,
        names: Arc<Mutex<BTreeMap<u32, String>>>,
        count: Arc<Mutex<Option<u32>>>,
        edits: Arc<Mutex<Vec<SegmentEdit>>>,
        /// `LIBERARR_DIARIZE=off`.
        diarize_off: bool,
    }
    impl Backend for Fake {
        async fn list(&self) -> Result<Vec<RecordingItem>> {
            Ok(vec![RecordingItem {
                id: 1,
                title: Some("t".into()),
                source: Source::File,
                duration_s: None,
                created_at: jiff::Timestamp::UNIX_EPOCH,
                status: RecordingStatus::Done,
                failed_step: None,
                no_speech: false,
            }])
        }
        async fn recording(&self, id: i64) -> Result<Option<Recording>> {
            Ok((id == 1).then(|| Recording {
                id,
                title: None,
                source: Source::File,
                source_ref: "r".into(),
                audio_path: "/nonexistent.ogg".into(),
                duration_s: None,
                created_at: jiff::Timestamp::UNIX_EPOCH,
                export_path: None,
            }))
        }
        /// Two segments: the first is covered by speaker 0, the second by nobody.
        async fn transcript(&self, _: i64) -> Result<Option<Transcript>> {
            let seg = |start, end| Segment {
                start,
                end,
                text: "x".into(),
            };
            Ok(Some(Transcript {
                engine: "e".into(),
                language: None,
                text: "x x".into(),
                segments: vec![seg(0.0, 5.0), seg(20.0, 25.0)],
                has_timestamps: true,
            }))
        }
        async fn transcript_with_edits(
            &self,
            id: i64,
        ) -> Result<Option<(Transcript, Vec<SegmentEdit>)>> {
            let t = self.transcript(id).await?;
            Ok(t.map(|t| (t, self.edits.lock().expect("lock").clone())))
        }
        async fn set_transcript_edits(&self, id: i64, changes: &[SegmentChange]) -> Result<()> {
            if id != 1 || changes.iter().any(|c| c.index > 1) {
                return Err(liberarr_core::Error::NotFound("segment".into()));
            }
            if changes.iter().any(|c| c.current != "x") {
                return Err(liberarr_core::Error::Conflict("segment".into()));
            }
            let mut edits = self.edits.lock().expect("lock");
            for c in changes {
                edits.push(SegmentEdit {
                    index: c.index,
                    original: "x".into(),
                    text: c.text.clone(),
                });
            }
            Ok(())
        }
        async fn summary_with_edit(
            &self,
            _: i64,
        ) -> Result<Option<(MeetingSummary, Option<SummaryText>)>> {
            Ok(None)
        }
        async fn set_summary_edit(
            &self,
            id: i64,
            generated: &SummaryText,
            _: &SummaryText,
        ) -> Result<()> {
            if id != 1 {
                return Err(liberarr_core::Error::NotFound("summary".into()));
            }
            if generated.title != "t" || generated.summary_md != "s" {
                return Err(liberarr_core::Error::Conflict("summary".into()));
            }
            Ok(())
        }
        /// Speakers 0 and 1 over the first ten seconds.
        async fn diarization(&self, id: i64) -> Result<Option<Diarization>> {
            let turn = |start, end, speaker| SpeakerTurn {
                start,
                end,
                speaker,
            };
            Ok((id == 1).then(|| Diarization {
                turns: vec![turn(0.0, 6.0, 0), turn(6.0, 10.0, 1)],
                engine: "fake".into(),
            }))
        }
        async fn speaker_names(&self, _: i64) -> Result<BTreeMap<u32, String>> {
            Ok(self.names.lock().expect("lock").clone())
        }
        async fn requested_speakers(&self, _: i64) -> Result<Option<u32>> {
            Ok(*self.count.lock().expect("lock"))
        }
        async fn set_speaker_name(&self, _: i64, speaker: u32, name: Option<&str>) -> Result<()> {
            let mut names = self.names.lock().expect("lock");
            match name {
                Some(n) => names.insert(speaker, n.into()),
                None => names.remove(&speaker),
            };
            Ok(())
        }
        async fn set_speaker_count(&self, _: i64, count: Option<u32>) -> Result<()> {
            if self.diarize_off {
                return Err(liberarr_core::Error::Conflict("diarization off".into()));
            }
            *self.count.lock().expect("lock") = count;
            Ok(())
        }
        async fn summary(&self, _: i64) -> Result<Option<MeetingSummary>> {
            Ok(None)
        }
        async fn jobs(&self, _: i64) -> Result<Vec<JobInfo>> {
            Ok(self.jobs.lock().expect("lock").clone())
        }
        async fn enqueue(&self, _: i64, step: &str) -> Result<i64> {
            let mut jobs = self.jobs.lock().expect("lock");
            let id = jobs.len() as i64 + 1;
            jobs.push(JobInfo {
                id,
                step: step.into(),
                status: JobStatus::Queued,
                attempts: 0,
                error: None,
                updated_at: jiff::Timestamp::UNIX_EPOCH,
            });
            Ok(id)
        }
        async fn toggle_action(
            &self,
            id: i64,
            index: usize,
            action: &str,
            done: bool,
        ) -> Result<MeetingSummary> {
            if id != 1 || index != 0 {
                return Err(liberarr_core::Error::NotFound("action".into()));
            }
            if action != "a" {
                return Err(liberarr_core::Error::Conflict("action".into()));
            }
            Ok(MeetingSummary {
                title: "t".into(),
                summary_md: "s".into(),
                mindmap: "mindmap".into(),
                actions: vec![Action {
                    action: "a".into(),
                    owner: None,
                    due: None,
                    start: None,
                    done,
                }],
            })
        }
    }

    fn app() -> Router {
        app_with(Fake::default())
    }

    /// Bound on all interfaces: no `Host` check (see `loopback_bind_refuses_foreign_host`).
    const ANY: SocketAddr =
        SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED), 0);

    fn app_with(fake: Fake) -> Router {
        app_on(fake, ANY)
    }

    fn app_on(fake: Fake, bind: SocketAddr) -> Router {
        router(
            AppState::new(
                fake,
                AuthConfig {
                    username: "admin".into(),
                    password: "pw".into(),
                    api_key: "key".into(),
                    secret_key: "secret".into(),
                    cookie_secure: false,
                },
            ),
            bind,
        )
    }

    fn req(method: &str, uri: &str, key: Option<&str>, body: &str) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(k) = key {
            b = b.header("x-api-key", k);
        }
        let mut r = b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_owned()))
            .expect("request");
        r.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9))));
        r
    }

    async fn status(app: &Router, r: Request<Body>) -> StatusCode {
        app.clone().oneshot(r).await.expect("response").status()
    }

    #[tokio::test]
    async fn api_requires_auth() {
        let app = app();
        assert_eq!(
            status(&app, req("GET", "/api/health", None, "")).await,
            StatusCode::OK
        );
        for uri in ["/api/recordings", "/api/recordings/1", "/api/auth/me"] {
            assert_eq!(
                status(&app, req("GET", uri, None, "")).await,
                StatusCode::UNAUTHORIZED
            );
        }
        // A forged or tampered session cookie is no better than none.
        let mut r = req("GET", "/api/recordings", None, "");
        r.headers_mut().insert(
            header::COOKIE,
            "liberarr_session=99999999999.deadbeef"
                .parse()
                .expect("header"),
        );
        assert_eq!(status(&app, r).await, StatusCode::UNAUTHORIZED);
        // A real login cookie opens the API.
        let login = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/auth/login",
                None,
                r#"{"username":"admin","password":"pw"}"#,
            ))
            .await
            .expect("response");
        assert_eq!(login.status(), StatusCode::NO_CONTENT);
        let set = login.headers()[header::SET_COOKIE]
            .to_str()
            .expect("cookie");
        assert!(set.contains("HttpOnly") && set.contains("SameSite=Lax"));
        let cookie = set.split(';').next().expect("pair").to_owned();
        let with_cookie = || {
            let mut r = req("GET", "/api/recordings", None, "");
            r.headers_mut()
                .insert(header::COOKIE, cookie.parse().expect("header"));
            r
        };
        assert_eq!(status(&app, with_cookie()).await, StatusCode::OK);
        // Same secret, new admin password: the old session is revoked.
        let rotated = router(
            AppState::new(
                Fake::default(),
                AuthConfig {
                    username: "admin".into(),
                    password: "changed".into(),
                    api_key: "key".into(),
                    secret_key: "secret".into(),
                    cookie_secure: false,
                },
            ),
            ANY,
        );
        assert_eq!(
            status(&rotated, with_cookie()).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn login_rate_limited_after_5_failures() {
        let app = app();
        let bad = r#"{"username":"admin","password":"nope"}"#;
        for _ in 0..5 {
            assert_eq!(
                status(&app, req("POST", "/api/auth/login", None, bad)).await,
                StatusCode::UNAUTHORIZED
            );
        }
        // Even the right password is refused once the budget is spent.
        let good = r#"{"username":"admin","password":"pw"}"#;
        assert_eq!(
            status(&app, req("POST", "/api/auth/login", None, good)).await,
            StatusCode::TOO_MANY_REQUESTS
        );
    }

    #[tokio::test]
    async fn logout_revokes_copied_session_cookie() {
        let app = app();
        let login = app
            .clone()
            .oneshot(req(
                "POST",
                "/api/auth/login",
                None,
                r#"{"username":"admin","password":"pw"}"#,
            ))
            .await
            .expect("response");
        let cookie = login.headers()[header::SET_COOKIE]
            .to_str()
            .expect("cookie")
            .split(';')
            .next()
            .expect("pair")
            .to_owned();
        let with_cookie = |method: &str, uri: &str| {
            let mut r = req(method, uri, None, "");
            r.headers_mut()
                .insert(header::COOKIE, cookie.parse().expect("header"));
            r
        };
        assert_eq!(
            status(&app, with_cookie("GET", "/api/recordings")).await,
            StatusCode::OK
        );
        assert_eq!(
            status(&app, with_cookie("POST", "/api/auth/logout")).await,
            StatusCode::NO_CONTENT
        );
        // A copy of the cookie kept from before the logout no longer opens the API.
        assert_eq!(
            status(&app, with_cookie("GET", "/api/recordings")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn login_rate_limit_spans_an_ipv6_64() {
        let app = app();
        let from = |ip: &str| {
            let mut r = req(
                "POST",
                "/api/auth/login",
                None,
                r#"{"username":"admin","password":"nope"}"#,
            );
            let peer = SocketAddr::new(ip.parse().expect("ip"), 9);
            r.extensions_mut().insert(ConnectInfo(peer));
            r
        };
        // Rotating the low 64 bits does not buy new attempts.
        for i in 0..5 {
            let ip = if i % 2 == 0 {
                "2001:db8::1"
            } else {
                "2001:db8::2"
            };
            assert_eq!(status(&app, from(ip)).await, StatusCode::UNAUTHORIZED);
        }
        assert_eq!(
            status(&app, from("2001:db8::3")).await,
            StatusCode::TOO_MANY_REQUESTS
        );
        // Another /64 keeps its own budget.
        assert_eq!(
            status(&app, from("2001:db8:0:1::1")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn api_key_header_authorizes() {
        let app = app();
        assert_eq!(
            status(&app, req("GET", "/api/recordings", Some("key"), "")).await,
            StatusCode::OK
        );
        assert_eq!(
            status(&app, req("GET", "/api/recordings", Some("wrong"), "")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    fn rerun(id: i64, step: &str) -> Request<Body> {
        req(
            "POST",
            &format!("/api/recordings/{id}/rerun"),
            Some("key"),
            &format!(r#"{{"step":"{step}"}}"#),
        )
    }

    #[tokio::test]
    async fn rerun_known_step_202_and_no_double_enqueue() {
        let fake = Fake::default();
        let jobs = Arc::clone(&fake.jobs);
        let app = app_with(fake);
        assert_eq!(
            status(&app, rerun(1, "bogus")).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(&app, rerun(2, "summarize")).await,
            StatusCode::NOT_FOUND
        );
        assert!(jobs.lock().expect("lock").is_empty());
        assert_eq!(
            status(&app, rerun(1, "summarize")).await,
            StatusCode::ACCEPTED
        );
        // Still queued: a second click is accepted but queues nothing.
        assert_eq!(
            status(&app, rerun(1, "summarize")).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(jobs.lock().expect("lock").len(), 1);
        // Once the job is done, a rerun queues again.
        jobs.lock().expect("lock")[0].status = JobStatus::Done;
        assert_eq!(
            status(&app, rerun(1, "summarize")).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(jobs.lock().expect("lock").len(), 2);
    }

    fn put(uri: &str, body: &str) -> Request<Body> {
        req("PUT", &format!("/api/recordings/{uri}"), Some("key"), body)
    }

    async fn json(app: &Router, r: Request<Body>) -> serde_json::Value {
        let res = app.clone().oneshot(r).await.expect("response");
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json")
    }

    #[tokio::test]
    async fn detail_labels_segments_and_lists_the_names() {
        let fake = Fake::default();
        *fake.count.lock().expect("lock") = Some(2);
        fake.names.lock().expect("lock").insert(0, "Alice".into());
        let d = json(
            &app_with(fake),
            req("GET", "/api/recordings/1", Some("key"), ""),
        )
        .await;
        assert_eq!(
            d["speakers"],
            serde_json::json!({"requested": 2, "names": {"0": "Alice"}})
        );
        let speakers: Vec<_> = d["transcript"]["segments"]
            .as_array()
            .expect("segments")
            .iter()
            .map(|s| s["speaker"].clone())
            .collect();
        assert_eq!(speakers, [serde_json::json!(0), serde_json::Value::Null]);
    }

    #[tokio::test]
    async fn rename_queues_export() {
        let fake = Fake::default();
        let (jobs, names) = (Arc::clone(&fake.jobs), Arc::clone(&fake.names));
        let app = app_with(fake);
        let rename = |n: u32, name: &str| {
            put(
                &format!("1/speakers/{n}"),
                &format!(r#"{{"name":{name:?}}}"#),
            )
        };
        assert_eq!(
            status(&app, rename(1, "  Bob ")).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(names.lock().expect("lock")[&1], "Bob");
        assert_eq!(jobs.lock().expect("lock")[0].step, "export");
        // Busy (the export is queued): the name is saved, no second export.
        assert_eq!(
            status(&app, rename(0, "Alice")).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(jobs.lock().expect("lock").len(), 1);
        // Blank resets.
        jobs.lock().expect("lock")[0].status = JobStatus::Done;
        assert_eq!(status(&app, rename(1, "  ")).await, StatusCode::NO_CONTENT);
        assert!(!names.lock().expect("lock").contains_key(&1));
        assert_eq!(jobs.lock().expect("lock").len(), 2);
        // Not a speaker of this recording, or no such recording.
        assert_eq!(status(&app, rename(2, "Eve")).await, StatusCode::NOT_FOUND);
        let other = put("2/speakers/0", r#"{"name":"Eve"}"#);
        assert_eq!(status(&app, other).await, StatusCode::NOT_FOUND);
        // 60 characters is the most; control characters are refused.
        assert_eq!(
            status(&app, rename(0, &"é".repeat(61))).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(&app, rename(0, "a\nb")).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status(&app, rename(0, &"é".repeat(60))).await,
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn speaker_count_refused_when_busy_or_off() {
        let fake = Fake::default();
        let (jobs, count) = (Arc::clone(&fake.jobs), Arc::clone(&fake.count));
        let app = app_with(fake);
        let set = |id: u32, body: &str| put(&format!("{id}/speaker-count"), body);
        for bad in [r#"{"count":0}"#, r#"{"count":21}"#, r#"{"count":-1}"#] {
            assert!(status(&app, set(1, bad)).await.is_client_error(), "{bad}");
        }
        assert_eq!(
            status(&app, set(2, r#"{"count":2}"#)).await,
            StatusCode::NOT_FOUND
        );
        assert!(jobs.lock().expect("lock").is_empty());
        assert_eq!(
            status(&app, set(1, r#"{"count":2}"#)).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(
            (
                *count.lock().expect("lock"),
                jobs.lock().expect("lock")[0].step.clone()
            ),
            (Some(2), "diarize".into())
        );
        // Busy: nothing is saved or queued.
        assert_eq!(
            status(&app, set(1, r#"{"count":5}"#)).await,
            StatusCode::CONFLICT
        );
        assert_eq!(*count.lock().expect("lock"), Some(2));
        assert_eq!(jobs.lock().expect("lock").len(), 1);
        // Done: null clears the count and 20 is the largest.
        jobs.lock().expect("lock")[0].status = JobStatus::Done;
        assert_eq!(
            status(&app, set(1, r#"{"count":null}"#)).await,
            StatusCode::ACCEPTED
        );
        assert_eq!(*count.lock().expect("lock"), None);
        jobs.lock().expect("lock")[1].status = JobStatus::Done;
        assert_eq!(
            status(&app, set(1, r#"{"count":20}"#)).await,
            StatusCode::ACCEPTED
        );

        // Diarization off: refused, nothing queued.
        let off = Fake {
            diarize_off: true,
            ..Fake::default()
        };
        let jobs = Arc::clone(&off.jobs);
        let app = app_with(off);
        assert_eq!(
            status(&app, set(1, r#"{"count":2}"#)).await,
            StatusCode::CONFLICT
        );
        assert!(jobs.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn patch_action_toggles_409_on_stale_text_404_out_of_range() {
        let app = app();
        let patch_text = |id: i64, index: usize, action: &str| {
            req(
                "PATCH",
                &format!("/api/recordings/{id}/actions/{index}"),
                Some("key"),
                &format!(r#"{{"action":"{action}","done":true}}"#),
            )
        };
        let patch = |id, index| patch_text(id, index, "a");
        assert_eq!(
            status(&app, patch_text(1, 0, "other")).await,
            StatusCode::CONFLICT
        );
        let res = app.clone().oneshot(patch(1, 0)).await.expect("response");
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        let actions: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(actions[0]["done"], true);
        assert_eq!(status(&app, patch(1, 5)).await, StatusCode::NOT_FOUND);
        assert_eq!(status(&app, patch(2, 0)).await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_api_is_404_not_spa() {
        let res = app()
            .oneshot(req("GET", "/api/nope", Some("key"), ""))
            .await
            .expect("response");
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    }

    #[tokio::test]
    async fn spa_fallback_serves_index() {
        let res = app()
            .oneshot(req("GET", "/recordings/42", None, ""))
            .await
            .expect("response");
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()["x-content-type-options"], "nosniff");
        let csp = res.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .expect("csp");
        assert!(csp.contains("script-src 'self';") && csp.contains("frame-ancestors 'none'"));
        let body = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("body");
        assert!(String::from_utf8_lossy(&body).contains("<html"));
    }

    fn edits_body(edits: serde_json::Value) -> String {
        serde_json::json!({ "edits": edits }).to_string()
    }

    fn one_edit(index: usize, current: &str, text: &str) -> String {
        edits_body(serde_json::json!([{ "index": index, "current": current, "text": text }]))
    }

    #[tokio::test]
    async fn transcript_edits_are_validated_and_guarded() {
        let app = app();
        let patch = |id: u32, body: String| {
            req(
                "PATCH",
                &format!("/api/recordings/{id}/transcript"),
                Some("key"),
                &body,
            )
        };
        let many: Vec<_> = (0..5001)
            .map(|i| serde_json::json!({"index": i, "current": "x", "text": "y"}))
            .collect();
        for bad in [
            edits_body(serde_json::json!([])),
            edits_body(serde_json::json!(many)),
            edits_body(serde_json::json!([
                {"index": 0, "current": "x", "text": "a"},
                {"index": 0, "current": "x", "text": "b"}
            ])),
            one_edit(0, "x", "a\u{7}b"),
            one_edit(0, "x", &"a".repeat(500_001)),
        ] {
            assert_eq!(status(&app, patch(1, bad)).await, StatusCode::BAD_REQUEST);
        }
        // A wrong shape never reaches our code: axum's extractor answers 422.
        let shape = r#"{"edits": [{"index": -1, "current": "x", "text": "y"}]}"#.to_string();
        assert_eq!(
            status(&app, patch(1, shape)).await,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            status(&app, patch(1, one_edit(0, "stale", "y"))).await,
            StatusCode::CONFLICT
        );
        assert_eq!(
            status(&app, patch(1, one_edit(9, "x", "y"))).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(&app, patch(2, one_edit(0, "x", "y"))).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(&app, patch(1, one_edit(0, "x", "line\nbreak"))).await,
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn an_edit_queues_one_export_and_waits_for_a_transcription() {
        let fake = Fake::default();
        let jobs = Arc::clone(&fake.jobs);
        let app = app_with(fake);
        let patch = || {
            req(
                "PATCH",
                "/api/recordings/1/transcript",
                Some("key"),
                &one_edit(0, "x", "y"),
            )
        };
        assert_eq!(status(&app, patch()).await, StatusCode::NO_CONTENT);
        assert_eq!(status(&app, patch()).await, StatusCode::NO_CONTENT);
        let steps: Vec<_> = jobs
            .lock()
            .expect("lock")
            .iter()
            .map(|j| j.step.clone())
            .collect();
        assert_eq!(steps, ["export"], "the queued export reads both edits");
        // A queued transcription will replace the transcript: refused.
        jobs.lock().expect("lock")[0].step = "transcribe".into();
        assert_eq!(status(&app, patch()).await, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn detail_shows_the_corrected_text_and_the_original() {
        let fake = Fake::default();
        *fake.edits.lock().expect("lock") = vec![
            SegmentEdit {
                index: 0,
                original: "x".into(),
                text: "y".into(),
            },
            SegmentEdit {
                index: 1,
                original: "x".into(),
                text: "".into(),
            },
        ];
        let d = json(
            &app_with(fake),
            req("GET", "/api/recordings/1", Some("key"), ""),
        )
        .await;
        let t = &d["transcript"];
        assert_eq!(t["text"], "y");
        assert_eq!(t["text_original"], serde_json::Value::Null);
        assert_eq!(t["segments"][0]["text"], "y");
        assert_eq!(t["segments"][0]["original"], "x");
        assert_eq!(t["segments"][0]["removed"], false);
        assert_eq!(t["segments"][1]["text"], "");
        assert_eq!(t["segments"][1]["removed"], true);
    }

    #[tokio::test]
    async fn summary_edit_is_validated_and_guarded() {
        let fake = Fake::default();
        let jobs = Arc::clone(&fake.jobs);
        let app = app_with(fake);
        let put = |title: &str, md: &str, generated_title: &str| {
            let body = serde_json::json!({"title": title, "summary_md": md,
                "generated_title": generated_title, "generated_summary_md": "s"});
            req(
                "PUT",
                "/api/recordings/1/summary",
                Some("key"),
                &body.to_string(),
            )
        };
        let (long_title, long_md) = ("t".repeat(201), "s".repeat(100_001));
        for (title, md) in [
            ("  ", "x"),
            (long_title.as_str(), "x"),
            ("a\nb", "x"),
            ("t2", long_md.as_str()),
            ("t2", "a\u{0}b"),
        ] {
            assert_eq!(
                status(&app, put(title, md, "t")).await,
                StatusCode::BAD_REQUEST
            );
        }
        assert_eq!(
            status(&app, put("New", "s\n| a\t|", "stale")).await,
            StatusCode::CONFLICT
        );
        assert_eq!(
            status(&app, put("New", "s\n| a\t|", "t")).await,
            StatusCode::NO_CONTENT
        );
        assert_eq!(jobs.lock().expect("lock")[0].step, "export");
        // A queued summary will replace it: refused.
        jobs.lock().expect("lock")[0].step = "summarize".into();
        assert_eq!(
            status(&app, put("New", "s", "t")).await,
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn loopback_bind_refuses_foreign_host() {
        let app = app_on(Fake::default(), SocketAddr::from(([127, 0, 0, 1], 7777)));
        let with_host = |host: Option<&str>| {
            let mut r = req("GET", "/api/health", None, "");
            if let Some(h) = host {
                r.headers_mut()
                    .insert(header::HOST, h.parse().expect("header"));
            }
            r
        };
        for host in ["127.0.0.1:7777", "localhost:7777", "[::1]:7777"] {
            assert_eq!(status(&app, with_host(Some(host))).await, StatusCode::OK);
        }
        // A rebound DNS name, the wrong port, or no Host at all.
        for host in [Some("evil.example:7777"), Some("localhost:8888"), None] {
            assert_eq!(
                status(&app, with_host(host)).await,
                StatusCode::MISDIRECTED_REQUEST
            );
        }
        // Not a loopback bind: any Host is served.
        let open = app_on(Fake::default(), ANY);
        assert_eq!(
            status(&open, with_host(Some("evil.example:7777"))).await,
            StatusCode::OK
        );
    }
}
