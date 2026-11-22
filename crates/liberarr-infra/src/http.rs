//! Shared POST and GET for the AI adapters. Errors carry the HTTP status and a short provider message,
//! never bodies, audio, text or the key. Rate limits (429), overload (503, 529) and connect or
//! timeout failures are retried here, at most [`ATTEMPTS`] times in all.
use liberarr_core::{Error, Result};
use reqwest::header::HeaderMap;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use std::hash::{BuildHasher, RandomState};
use std::time::Duration;

const ATTEMPTS: u32 = 3;
/// A provider asking for a longer wait is not worth holding a job for: fail at once.
const MAX_WAIT: Duration = Duration::from_secs(60);
/// First backoff when the provider gives no hint; doubles on each attempt. Tiny in tests.
const BASE_DELAY: Duration = if cfg!(test) {
    Duration::from_millis(1)
} else {
    Duration::from_secs(2)
};

#[derive(Clone, Copy)]
pub(crate) enum Auth<'a> {
    Bearer(&'a str),
}

/// No redirects: a hostile or misconfigured base URL must not forward the key or the audio.
/// A 3xx then reaches `send` as an error.
pub(crate) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest client")
}

/// POSTs the request, maps failures to `Error::Provider` and returns the parsed JSON reply.
/// `req` adds the body and runs once per attempt. `retry_timeout: false` for long uploads
/// (transcription): after a timeout the provider may still be working, and billing, on it.
pub(crate) async fn send(
    http: &reqwest::Client,
    url: &str,
    auth: Auth<'_>,
    timeout: Duration,
    retry_timeout: bool,
    req: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> Result<Value> {
    exchange(http, Method::POST, url, auth, timeout, retry_timeout, req).await
}

/// Same rules as [`send`], for a GET.
pub(crate) async fn get(
    http: &reqwest::Client,
    url: &str,
    auth: Auth<'_>,
    timeout: Duration,
    req: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> Result<Value> {
    exchange(http, Method::GET, url, auth, timeout, true, req).await
}

/// One failed attempt: worth another one (after the provider's wait, if it gave one), or final.
enum Failure {
    Retry(Option<Duration>, String),
    Fatal(Error),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Fatal(e)
    }
}

async fn exchange(
    http: &reqwest::Client,
    method: Method,
    url: &str,
    auth: Auth<'_>,
    timeout: Duration,
    retry_timeout: bool,
    req: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> Result<Value> {
    let mut attempt = 1;
    loop {
        match attempt_once(
            http,
            method.clone(),
            url,
            auth,
            timeout,
            retry_timeout,
            &req,
        )
        .await
        {
            Ok(v) => return Ok(v),
            Err(Failure::Fatal(e)) => return Err(e),
            Err(Failure::Retry(wait, _)) if attempt < ATTEMPTS => {
                tokio::time::sleep(wait.unwrap_or_else(|| backoff(attempt))).await;
                attempt += 1;
            }
            Err(Failure::Retry(_, text)) => {
                return Err(Error::Provider(format!(
                    "{text} (after {ATTEMPTS} attempts)"
                )));
            }
        }
    }
}

/// `BASE_DELAY * 2^(attempt-1)`, scaled by a random factor in [0.5, 1) so parallel chunks spread out.
fn backoff(attempt: u32) -> Duration {
    let r = RandomState::new().hash_one(attempt) % 1000;
    (BASE_DELAY * 2u32.pow(attempt - 1)).mul_f64(0.5 + r as f64 / 2000.0)
}

/// `retry-after-ms` (OpenAI), else `Retry-After` in seconds or as an HTTP date.
fn retry_hint(h: &HeaderMap) -> Option<Duration> {
    let header = |name| h.get(name)?.to_str().ok().map(str::trim);
    if let Some(ms) = header("retry-after-ms").and_then(|v| v.parse::<f64>().ok()) {
        return Duration::try_from_secs_f64(ms / 1000.0).ok();
    }
    let v = header("retry-after")?;
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = jiff::fmt::rfc2822::DateTimeParser::new()
        .parse_timestamp(v)
        .ok()?;
    let secs = at.as_second() - jiff::Timestamp::now().as_second();
    Some(Duration::from_secs(secs.max(0).unsigned_abs()))
}

/// A 429 that means "out of money or quota", not "too fast": waiting will not help.
fn is_spend_cap(json: Option<&Value>) -> bool {
    let Some(j) = json else { return false };
    let code = ["/error/code", "/error/type", "/code", "/type"]
        .iter()
        .filter_map(|p| j.pointer(p)?.as_str())
        .any(|c| c == "insufficient_quota" || c == "billing_hard_limit_reached");
    let msg = ["/error/message", "/message"]
        .iter()
        .filter_map(|p| j.pointer(p)?.as_str())
        .any(|m| {
            let m = m.to_lowercase();
            ["quota", "billing", "credit", "spend"]
                .iter()
                .any(|w| m.contains(w))
        });
    code || msg
}

async fn attempt_once(
    http: &reqwest::Client,
    method: Method,
    url: &str,
    auth: Auth<'_>,
    timeout: Duration,
    retry_timeout: bool,
    req: &impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
) -> std::result::Result<Value, Failure> {
    let builder = http.request(method, url).timeout(timeout);
    let Auth::Bearer(key) = auth;
    let builder = builder.bearer_auth(key);
    let res = req(builder).send().await.map_err(|e| {
        // The provider was never reached, or (when allowed) gave no reply in time.
        let transient = e.is_connect() || (retry_timeout && e.is_timeout());
        let text = format!("request failed: {}", e.without_url());
        if transient {
            Failure::Retry(None, text)
        } else {
            Failure::Fatal(Error::Provider(text))
        }
    })?;
    let status = res.status();
    let hint = retry_hint(res.headers());
    let body = res.text().await.map_err(|e| {
        Error::Provider(format!(
            "HTTP {status}: unreadable reply: {}",
            e.without_url()
        ))
    })?;
    let json: Option<Value> = serde_json::from_str(&body).ok();
    let msg = json
        .as_ref()
        // `/error/message` (OpenAI, Anthropic) or a top-level `/message` (Mistral).
        .and_then(|j| {
            j.pointer("/error/message")
                .or_else(|| j.pointer("/message"))?
                .as_str()
        })
        .filter(|m| {
            let m = m.to_lowercase();
            !["key", "token", "bearer", "sk-"]
                .iter()
                .any(|w| m.contains(w))
        })
        .map(|m| format!(": {}", m.chars().take(200).collect::<String>()))
        .unwrap_or_default();
    // 529: Anthropic's "overloaded".
    if matches!(status.as_u16(), 429 | 503 | 529) {
        let text = format!("HTTP {status}{msg}");
        return Err(match hint {
            Some(w) if w > MAX_WAIT => Failure::Fatal(Error::Provider(format!(
                "{text}: provider asks to wait {} s, over the {} s limit",
                w.as_secs(),
                MAX_WAIT.as_secs()
            ))),
            None if status == StatusCode::TOO_MANY_REQUESTS && is_spend_cap(json.as_ref()) => {
                Failure::Fatal(Error::Provider(format!(
                    "{text}: quota or spend limit reached, check the provider account"
                )))
            }
            _ => Failure::Retry(hint, text),
        });
    }
    // Providers quote a masked key in these messages: keep only the status. A 403 can also be a
    // plan or region refusal: its short message (filtered above) is then more useful.
    if status == StatusCode::UNAUTHORIZED || (status == StatusCode::FORBIDDEN && msg.is_empty()) {
        return Err(
            Error::Provider(format!("provider refused the API key (HTTP {status})")).into(),
        );
    }
    if status.is_redirection() {
        // Never echo `Location`: it can carry anything.
        return Err(Error::Provider("redirect refused: check the base URL (https?)".into()).into());
    }
    if !status.is_success() {
        return Err(Error::Provider(format!("HTTP {status}{msg}")).into());
    }
    json.ok_or_else(|| Error::Provider(format!("HTTP {status}: reply is not JSON")).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::HeaderValue, response::IntoResponse, routing::get};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Serves `reply(n)` for the n-th call (from 1) and counts the calls.
    async fn serve(
        reply: impl Fn(u32) -> axum::response::Response + Clone + Send + Sync + 'static,
    ) -> (String, Arc<AtomicU32>) {
        let hits = Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        let app = Router::new().route(
            "/",
            get(move || {
                let n = h.fetch_add(1, Ordering::SeqCst) + 1;
                let r = reply(n);
                async move { r }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}/"), hits)
    }

    fn limited(
        header: (&'static str, &'static str),
        body: &'static str,
    ) -> axum::response::Response {
        let mut r = (axum::http::StatusCode::TOO_MANY_REQUESTS, body).into_response();
        if !header.0.is_empty() {
            r.headers_mut()
                .insert(header.0, HeaderValue::from_static(header.1));
        }
        r
    }

    async fn call(url: &str) -> Result<Value> {
        super::get(&client(), url, Auth::Bearer("k"), MAX_WAIT, |b| b).await
    }

    #[tokio::test]
    async fn retries_after_the_hint_then_succeeds() {
        let (url, hits) = serve(|n| match n {
            1 => limited(("retry-after", "0"), "{}"),
            2 => limited(("retry-after-ms", "1"), "{}"),
            _ => r#"{"ok":true}"#.into_response(),
        })
        .await;
        assert_eq!(call(&url).await.unwrap()["ok"], true);
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn gives_up_after_three_attempts() {
        let (url, hits) =
            serve(|_| limited(("", ""), r#"{"error":{"message":"slow down"}}"#)).await;
        let err = call(&url).await.unwrap_err().to_string();
        assert!(
            err.contains("429") && err.contains("after 3 attempts"),
            "{err}"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_wait_over_the_cap_fails_at_once() {
        let (url, hits) = serve(|_| limited(("retry-after", "3600"), "{}")).await;
        let err = call(&url).await.unwrap_err().to_string();
        assert!(err.contains("over the 60 s limit"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_spend_cap_fails_at_once() {
        let (url, hits) = serve(|_| {
            limited(
                ("", ""),
                r#"{"error":{"message":"You exceeded your current quota.","code":"insufficient_quota"}}"#,
            )
        })
        .await;
        let err = call(&url).await.unwrap_err().to_string();
        assert!(err.contains("quota or spend limit"), "{err}");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_timeout_is_retried_only_when_allowed() {
        // Accepts connections and never replies.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicU32::new(0));
        let h = hits.clone();
        tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((s, _)) = listener.accept().await {
                h.fetch_add(1, Ordering::SeqCst);
                open.push(s);
            }
        });
        let short = Duration::from_millis(50);
        for (retry, calls) in [(false, 1), (true, 3)] {
            hits.store(0, Ordering::SeqCst);
            let err = send(&client(), &url, Auth::Bearer("k"), short, retry, |b| b).await;
            assert!(err.is_err());
            assert_eq!(hits.load(Ordering::SeqCst), calls, "retry_timeout: {retry}");
        }
    }

    #[test]
    fn reads_an_http_date_hint() {
        let mut h = HeaderMap::new();
        h.insert(
            "retry-after",
            HeaderValue::from_static("Wed, 21 Oct 2015 07:28:00 GMT"),
        );
        assert_eq!(retry_hint(&h), Some(Duration::ZERO));
    }
}
