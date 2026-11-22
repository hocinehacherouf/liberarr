//! Lists the models an account can use: doubles as the connection test (no token spent).
//! Errors follow `http`: status and a short message, never a body or the key.
use crate::http::{self, Auth};
use liberarr_core::ai::Provider;
use liberarr_core::{Error, Result};
use serde_json::Value;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(15);

/// `GET {base}/models` (bearer); OpenRouter: `GET {base}/key`, empty list on success. Reads
/// `data[].id`; any other JSON shape gives an empty list. The `claude` CLI has no HTTP API.
pub async fn list_models(provider: Provider, base_url: &str, api_key: &str) -> Result<Vec<String>> {
    let base = base_url.trim_end_matches('/');
    let http = http::client();
    let reply = match provider {
        Provider::Claude | Provider::Codex | Provider::Copilot => {
            return Err(Error::Provider(format!(
                "the {} CLI has no model list",
                provider.as_str()
            )));
        }
        Provider::Local => return Err(Error::Provider("local has no model list".into())),
        // `/models` is public (it proves nothing about the key) and lists 400+ entries: test the
        // key on `/key` and offer nothing to pick from.
        Provider::OpenRouter => {
            http::get(
                &http,
                &format!("{base}/key"),
                Auth::Bearer(api_key),
                TIMEOUT,
                |b| b,
            )
            .await?;
            return Ok(Vec::new());
        }
        _ => {
            http::get(
                &http,
                &format!("{base}/models"),
                Auth::Bearer(api_key),
                TIMEOUT,
                |b| b,
            )
            .await?
        }
    };
    Ok(reply
        .get("data")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("id")?.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// The providers that serve an OpenRouter model, as routing slugs (`azure`, `mistral`), sorted.
/// `GET {base}/models/{model}/endpoints`.
pub async fn openrouter_providers(
    base_url: &str,
    api_key: &str,
    model: &str,
) -> Result<Vec<String>> {
    let base = base_url.trim_end_matches('/');
    let reply = http::get(
        &http::client(),
        &format!("{base}/models/{model}/endpoints"),
        Auth::Bearer(api_key),
        TIMEOUT,
        |b| b,
    )
    .await?;
    // A tag is `slug` or `slug/variant` (`azure/eu`): the slug covers every variant.
    let mut slugs: Vec<String> = reply
        .pointer("/data/endpoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e.get("tag")?.as_str()?.split('/').next().map(String::from))
        .filter(|s| !s.is_empty())
        .collect();
    slugs.sort();
    slugs.dedup();
    Ok(slugs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::Request, http::StatusCode, routing::get};

    const KEY: &str = "sk-test-secret";

    async fn serve() -> String {
        let app = Router::new()
            .route(
                "/ok/models",
                get(|req: Request| async move {
                    let ok = req
                        .headers()
                        .get("authorization")
                        .is_some_and(|a| a == format!("Bearer {KEY}").as_str());
                    if ok {
                        (
                            StatusCode::OK,
                            r#"{"data":[{"id":"a"},{"id":"b"},{"x":1}]}"#,
                        )
                    } else {
                        (StatusCode::BAD_REQUEST, "{}")
                    }
                }),
            )
            .route(
                "/other/models",
                get(|| async { (StatusCode::OK, r#"{"models":["a"]}"#) }),
            )
            .route(
                "/denied/models",
                get(|| async {
                    (
                        StatusCode::UNAUTHORIZED,
                        r#"{"error":{"message":"Incorrect API key provided: sk-...abcd."}}"#,
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn lists_ids_and_tolerates_other_shapes() {
        let base = serve().await;
        let ids = list_models(Provider::Mistral, &format!("{base}/ok/"), KEY).await;
        assert_eq!(ids.unwrap(), ["a", "b"]);
        let ids = list_models(Provider::Compatible, &format!("{base}/other"), KEY).await;
        assert!(ids.unwrap().is_empty());
    }

    #[tokio::test]
    async fn refused_key_gives_the_fixed_text_without_the_key() {
        let base = serve().await;
        let err = list_models(Provider::Compatible, &format!("{base}/denied"), KEY)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("provider refused the API key"), "{err}");
        assert!(!err.contains("sk-...abcd") && !err.contains(KEY), "{err}");
    }
}
