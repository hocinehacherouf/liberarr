//! What `init` proposes for each provider.
use liberarr_core::ai::{Provider, Step};

const OPENAI_URL: &str = "https://api.openai.com/v1";
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1";

/// `(default base URL, default model)`. Summary models have no default: they change too fast.
pub fn defaults(p: Provider, step: Step) -> (Option<&'static str>, Option<&'static str>) {
    match (p, step) {
        (Provider::Compatible, Step::Transcribe) => (Some(OPENAI_URL), Some("whisper-1")),
        (Provider::Compatible, Step::Summarize) => (Some(OPENAI_URL), None),
        (Provider::Mistral, Step::Transcribe) => (
            Some("https://api.mistral.ai/v1"),
            Some("voxtral-mini-transcribe-3"),
        ),
        (Provider::Mistral, Step::Summarize) => (Some("https://api.mistral.ai/v1"), None),
        (Provider::OpenRouter, Step::Transcribe) => (
            Some(OPENROUTER_URL),
            Some("mistralai/voxtral-mini-transcribe-3-0"),
        ),
        (Provider::OpenRouter, Step::Summarize) => (Some(OPENROUTER_URL), None),
        (Provider::Claude, _) => (None, Some("sonnet")),
        (Provider::Codex | Provider::Copilot, _) => (None, None),
    }
}
