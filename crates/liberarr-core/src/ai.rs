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
}

impl Provider {
    /// `openai` is an old name of `openai-compatible`. Anything else, `anthropic` included, is `None`.
    pub fn parse(s: &str) -> Option<Provider> {
        Some(match s {
            "openai-compatible" | "openai" => Provider::Compatible,
            "mistral" => Provider::Mistral,
            "openrouter" => Provider::OpenRouter,
            "claude" => Provider::Claude,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Compatible => "openai-compatible",
            Provider::Mistral => "mistral",
            Provider::OpenRouter => "openrouter",
            Provider::Claude => "claude",
        }
    }

    /// A local command-line tool with its own login, not an HTTP API.
    pub fn is_cli(self) -> bool {
        matches!(self, Provider::Claude)
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
