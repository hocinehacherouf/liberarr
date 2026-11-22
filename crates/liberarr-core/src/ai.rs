//! AI providers and which steps each can do. Pure data: no I/O.

/// Transcription APIs cap uploads at 25 MB; stay under.
pub const DEFAULT_UPLOAD_BYTES: usize = 24 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    /// Any OpenAI-compatible base URL (OpenAI itself, or a gateway such as LiteLLM).
    Compatible,
    Mistral,
    /// OpenAI-compatible chat through the user's own provider keys (BYOK).
    OpenRouter,
    /// The user's logged-in `claude` CLI (subscription), summaries only.
    Claude,
    /// The user's logged-in `codex` CLI, summaries only.
    Codex,
    /// The user's logged-in GitHub `copilot` CLI, summaries only.
    Copilot,
}

impl Provider {
    /// `openai` is an old name of `openai-compatible`. Anything else, `anthropic` included, is `None`.
    pub fn parse(s: &str) -> Option<Provider> {
        Some(match s {
            "openai-compatible" | "openai" => Provider::Compatible,
            "mistral" => Provider::Mistral,
            "openrouter" => Provider::OpenRouter,
            "claude" => Provider::Claude,
            "codex" => Provider::Codex,
            "copilot" => Provider::Copilot,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Compatible => "openai-compatible",
            Provider::Mistral => "mistral",
            Provider::OpenRouter => "openrouter",
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Copilot => "copilot",
        }
    }

    /// A local command-line tool with its own login, not an HTTP API.
    pub fn is_cli(self) -> bool {
        matches!(self, Provider::Claude | Provider::Codex | Provider::Copilot)
    }

    /// CLIs summarize only; the others do both steps.
    pub fn can(self, step: Step) -> bool {
        match step {
            Step::Transcribe => !self.is_cli(),
            Step::Summarize => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Transcribe,
    Summarize,
}
