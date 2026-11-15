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
use axum::routing::{get, patch, post};
use liberarr_core::Result;
use liberarr_core::jobs::JobInfo;
use liberarr_core::recordings::{Recording, RecordingItem};
use liberarr_core::summaries::MeetingSummary;
use liberarr_core::transcription::Transcript;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

pub use auth::{Auth, AuthConfig};

pub trait Backend: Send + Sync + 'static {
    fn list(&self) -> impl Future<Output = Result<Vec<RecordingItem>>> + Send;
    fn recording(&self, id: i64) -> impl Future<Output = Result<Option<Recording>>> + Send;
    fn transcript(&self, id: i64) -> impl Future<Output = Result<Option<Transcript>>> + Send;
    fn summary(&self, id: i64) -> impl Future<Output = Result<Option<MeetingSummary>>> + Send;
    fn jobs(&self, id: i64) -> impl Future<Output = Result<Vec<JobInfo>>> + Send;
    /// Queues `step` (already validated against `STEPS`); the worker picks it up.
    fn enqueue(&self, id: i64, step: &str) -> impl Future<Output = Result<i64>> + Send;
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
            "/recordings/{id}/actions/{index}",
            patch(api::toggle_action::<B>),
        )
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
    use liberarr_core::jobs::JobStatus;
    use liberarr_core::recordings::{RecordingStatus, Source};
    use liberarr_core::summaries::Action;
    use std::sync::Mutex;
    use tower::ServiceExt;

    /// One recording (id 1) whose summary has a single action.
    #[derive(Default)]
    struct Fake {
        jobs: Arc<Mutex<Vec<JobInfo>>>,
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
        async fn transcript(&self, _: i64) -> Result<Option<Transcript>> {
            Ok(None)
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
