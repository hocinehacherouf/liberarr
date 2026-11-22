//! OpenAI-compatible HTTP client: transcription and chat completions against any base URL.
//! Errors carry the HTTP status and a short provider message, never bodies, audio, text or the key.
use crate::http::{self, Auth};
use liberarr_core::summaries::{ChatMessage, Llm};
use liberarr_core::transcription::{ChunkResult, Segment, Transcriber};
use liberarr_core::{Error, Result};
use reqwest::multipart::{Form, Part};
use serde_json::{Value, json};
use std::time::Duration;

const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(600);
const CHAT_TIMEOUT: Duration = Duration::from_secs(180);

/// One profile (base URL, key, model). Build one for `transcribe` and one for `llm`.
pub struct OpenAiCompat {
    base_url: String,
    api_key: String,
    model: String,
    /// The `provider` object of every request (OpenRouter routing), when set.
    routing: Option<Value>,
    http: reqwest::Client,
}

impl OpenAiCompat {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            routing: None,
            http: http::client(),
        }
    }

    /// Requests only go to these providers, with no fallback: OpenRouter's own shared models
    /// stay out of reach (`only` is the real guard; `allow_fallbacks: false` alone is not enough).
    pub fn with_routing(mut self, only: &[String]) -> Self {
        self.routing =
            Some(json!({"only": only, "allow_fallbacks": false, "require_parameters": true}));
        self
    }

    async fn send(
        &self,
        path: &str,
        timeout: Duration,
        retry_timeout: bool,
        req: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    ) -> Result<Value> {
        let url = format!("{}/{path}", self.base_url);
        let auth = Auth::Bearer(&self.api_key);
        http::send(&self.http, &url, auth, timeout, retry_timeout, req).await
    }
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64()
}

/// Reads `text`, `language` and the optional `segments` of a transcription reply.
pub(crate) fn chunk_result(r: &Value) -> Result<ChunkResult> {
    let text = r
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Provider("reply has no `text`".into()))?;
    // Segments are optional: some providers return plain text only.
    let segments = r
        .get("segments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(Segment {
                        start: num(s, "start")?,
                        end: num(s, "end")?,
                        text: s.get("text")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ChunkResult {
        // Some models reply with a `languages` array of `{code}` objects instead; some
        // compatible gateways send plain strings.
        language: r
            .get("language")
            .or_else(|| r.pointer("/languages/0/code"))
            .or_else(|| r.pointer("/languages/0"))
            .and_then(Value::as_str)
            .map(String::from),
        text: text.to_string(),
        segments,
    })
}

impl Transcriber for OpenAiCompat {
    async fn transcribe(&self, audio: &[u8], filename: &str) -> Result<ChunkResult> {
        // Built per attempt: a multipart body cannot be cloned for a retry.
        let form = || {
            // No `language`: recordings mix French and English.
            let form = Form::new()
                .text("model", self.model.clone())
                .text("response_format", "verbose_json")
                .part(
                    "file",
                    Part::bytes(audio.to_vec()).file_name(filename.to_string()),
                );
            // Multipart takes the routing object as a JSON-encoded field.
            match &self.routing {
                Some(routing) => form.text("provider", routing.to_string()),
                None => form,
            }
        };
        let r = self
            .send("audio/transcriptions", TRANSCRIBE_TIMEOUT, false, |b| {
                b.multipart(form())
            })
            .await?;
        chunk_result(&r)
    }

    fn engine(&self) -> &str {
        &self.model
    }
}

impl Llm for OpenAiCompat {
    async fn complete_json(&self, messages: &[ChatMessage]) -> Result<String> {
        let mut body = json!({
            "model": self.model,
            "messages": messages.iter().map(|m| json!({"role": m.role.as_str(), "content": m.content})).collect::<Vec<_>>(),
            "response_format": {"type": "json_object"},
        });
        if let Some(routing) = &self.routing {
            body["provider"] = routing.clone();
        }
        let r = self
            .send("chat/completions", CHAT_TIMEOUT, true, |b| {
                b.header("content-type", "application/json")
                    .body(body.to_string())
            })
            .await?;
        match r
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            Some("content_filter") => {
                return Err(Error::Provider(
                    "reply cut by the provider's content filter".into(),
                ));
            }
            Some("length") => {
                return Err(Error::Provider("reply cut at the token limit".into()));
            }
            _ => {}
        }
        r.pointer("/choices/0/message/content")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(|| Error::Provider("reply has no message content".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::Request, http::StatusCode, routing::post};
    use liberarr_core::Error;
    use liberarr_core::transcription::Transcriber;

    const KEY: &str = "sk-test-secret";

    /// Fails (400) unless the request is the documented multipart shape: bearer key, verbose_json, no language.
    async fn transcription(req: Request, reply: &'static str) -> (StatusCode, String) {
        let auth = req.headers().get("authorization").cloned();
        let body = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        let ok = auth.is_some_and(|a| a == format!("Bearer {KEY}").as_str())
            && body.contains("name=\"model\"")
            && body.contains("verbose_json")
            && body.contains("name=\"file\"; filename=\"chunk-0.ogg\"")
            && !body.contains("name=\"language\"");
        if ok {
            (StatusCode::OK, reply.into())
        } else {
            (StatusCode::BAD_REQUEST, "bad request".into())
        }
    }

    async fn serve() -> String {
        let app = Router::new()
            .route(
                "/seg/audio/transcriptions",
                post(|req: Request| {
                    transcription(
                        req,
                        r#"{"language":"french","text":" Bonjour. Hello. ","segments":[{"start":0.0,"end":1.5,"text":" Bonjour."},{"start":1.5,"end":3.0,"text":" Hello."}]}"#,
                    )
                }),
            )
            .route(
                "/plain/audio/transcriptions",
                post(|req: Request| {
                    transcription(req, r#"{"text":"Bonjour","languages":[{"code":"fr"}]}"#)
                }),
            )
            .route(
                "/html/audio/transcriptions",
                post(|| async { (StatusCode::OK, "<html>oops</html>") }),
            )
            .route(
                "/filter/chat/completions",
                post(|| async {
                    (
                        StatusCode::OK,
                        r#"{"choices":[{"finish_reason":"content_filter","message":{"content":"{\"half\":"}}]}"#,
                    )
                }),
            )
            .route(
                "/length/chat/completions",
                post(|| async {
                    (
                        StatusCode::OK,
                        r#"{"choices":[{"finish_reason":"length","message":{"content":"{\"half\":"}}]}"#,
                    )
                }),
            )
            .route(
                "/echo/chat/completions",
                // Replies with the `provider` field it received, as the message content.
                post(|body: String| async move {
                    let v: Value = serde_json::from_str(&body).unwrap();
                    let seen = v.get("provider").cloned().unwrap_or(Value::Null);
                    json!({"choices":[{"message":{"content": seen.to_string()}}]}).to_string()
                }),
            )
            .route(
                "/echo/audio/transcriptions",
                // Replies with the `provider` part it received (`null` if none), as the text.
                post(|body: String| async move {
                    let seen = body
                        .split_once("name=\"provider\"\r\n\r\n")
                        .and_then(|(_, rest)| rest.split_once("\r\n"))
                        .map_or("null", |(v, _)| v)
                        .to_string();
                    json!({ "text": seen }).to_string()
                }),
            )
            .route(
                "/denied/audio/transcriptions",
                post(|| async { (StatusCode::UNAUTHORIZED, r#"{"error":{"message":"Incorrect API key provided: sk-...abcd."}}"#) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn transcribe_parses_verbose_json_and_missing_segments() {
        let base = serve().await;
        let client =
            |p: &str| OpenAiCompat::new(format!("{base}/{p}/"), KEY.into(), "whisper-1".into());

        let r = client("seg")
            .transcribe(b"audio", "chunk-0.ogg")
            .await
            .unwrap();
        assert_eq!(r.language.as_deref(), Some("french"));
        assert_eq!(r.text, " Bonjour. Hello. ");
        assert_eq!(r.segments.len(), 2);
        assert_eq!((r.segments[1].start, r.segments[1].end), (1.5, 3.0));

        let r = client("plain")
            .transcribe(b"audio", "chunk-0.ogg")
            .await
            .unwrap();
        assert_eq!(
            (r.language.as_deref(), r.text.as_str(), r.segments.len()),
            (Some("fr"), "Bonjour", 0)
        );

        let err = client("denied")
            .transcribe(b"audio", "chunk-0.ogg")
            .await
            .unwrap_err();
        assert!(matches!(&err, Error::Provider(m) if m.contains("401")));
        // The provider quotes a masked key: it must not reach job.error or the UI.
        assert!(!err.to_string().contains("sk-...abcd"), "{err}");
        assert!(!err.to_string().contains(KEY));
        assert_eq!(client("seg").engine(), "whisper-1");
    }

    #[tokio::test]
    async fn cut_reply_is_an_error() {
        let base = serve().await;
        for (path, expected) in [("filter", "content filter"), ("length", "token limit")] {
            let llm = OpenAiCompat::new(format!("{base}/{path}"), KEY.into(), "m".into());
            let msg = llm.complete_json(&[]).await.unwrap_err().to_string();
            assert!(msg.contains(expected), "{msg}");
        }
    }

    /// Privacy and money guard: with routing, chat and transcription requests name the allowed
    /// providers and forbid fallback; without it, no `provider` field is sent.
    #[tokio::test]
    async fn routing_is_sent_only_when_set() {
        let base = serve().await;
        let client = OpenAiCompat::new(format!("{base}/echo"), KEY.into(), "m".into());
        assert_eq!(client.complete_json(&[]).await.unwrap(), "null");
        assert_eq!(client.transcribe(b"a", "c.ogg").await.unwrap().text, "null");
        let routed = client.with_routing(&["azure".to_string()]);
        let chat = routed.complete_json(&[]).await.unwrap();
        let stt = routed.transcribe(b"a", "c.ogg").await.unwrap().text;
        for seen in [chat, stt] {
            let seen: Value = serde_json::from_str(&seen).unwrap();
            assert_eq!(seen["only"], json!(["azure"]));
            assert_eq!(seen["allow_fallbacks"], json!(false));
            assert_eq!(seen["require_parameters"], json!(true));
        }
    }

    #[tokio::test]
    async fn non_json_reply_is_an_error_not_a_panic() {
        let base = serve().await;
        let t = OpenAiCompat::new(format!("{base}/html"), KEY.into(), "m".into());
        assert!(t.transcribe(b"audio", "chunk-0.ogg").await.is_err());
    }
}
