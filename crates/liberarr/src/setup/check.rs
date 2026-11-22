//! The connection test behind `init`: the model list of an API provider, or one trivial message
//! through an AI CLI.
use liberarr_core::ai::Provider;
use liberarr_core::summaries::{ChatMessage, Llm, Role};
use liberarr_infra::agent_cli::{AgentCli, Preset};
use liberarr_infra::model_list::{list_models, openrouter_providers};
use std::path::PathBuf;
use std::time::Duration;

/// A first call can be slow (login refresh, model start).
const CLI_TIMEOUT: Duration = Duration::from_secs(60);

/// The [`super::Checker`] of the real command: blocks on `rt`. For a CLI, `url` is the path of
/// the program (empty: found by name) and `oauth` the token of a server login, if any; any reply is
/// a success. The error is a fixed text: neither the reply nor a token is ever in it.
pub fn connect(
    rt: &tokio::runtime::Runtime,
    provider: Provider,
    url: &str,
    key: &str,
    oauth: Option<&str>,
) -> Result<Vec<String>, String> {
    // Nothing to reach: the model is downloaded on the first transcription.
    if provider == Provider::Local {
        return Ok(Vec::new());
    }
    if let Some(preset) = Preset::from_provider(provider) {
        let program = (!url.is_empty()).then(|| PathBuf::from(url));
        let cli = AgentCli::new(preset, program, String::new(), oauth.map(String::from))
            .with_timeout(CLI_TIMEOUT);
        let hello = [ChatMessage {
            role: Role::User,
            content: r#"Reply with the JSON {"ok":true}"#.to_string(),
        }];
        return rt
            .block_on(cli.complete_json(&hello))
            .map(|_| Vec::new())
            .map_err(|_| format!("{} CLI did not answer", preset.program_name()));
    }
    rt.block_on(list_models(provider, url, key))
        .map_err(|e| e.to_string())
}

/// The [`super::Routes`] of the real command: blocks on `rt`.
pub fn routes(
    rt: &tokio::runtime::Runtime,
    url: &str,
    key: &str,
    model: &str,
) -> Result<Vec<String>, String> {
    rt.block_on(openrouter_providers(url, key, model))
        .map_err(|e| e.to_string())
}
