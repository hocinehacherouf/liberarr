//! Curated models `init` proposes. Data only: edit the table when a model is released or retired.
use liberarr_core::ai::{Provider, Step};

pub struct Entry {
    pub provider: Provider,
    pub step: Step,
    pub id: &'static str,
    /// Catalog key of the short description.
    pub label: &'static str,
    pub recommended: bool,
    /// Retirement date (`YYYY-MM-DD`), shown next to the model.
    pub deprecated: Option<&'static str>,
}

const fn entry(
    provider: Provider,
    step: Step,
    id: &'static str,
    label: &'static str,
    recommended: bool,
    deprecated: Option<&'static str>,
) -> Entry {
    Entry {
        provider,
        step,
        id,
        label,
        recommended,
        deprecated,
    }
}

use Provider::{Claude, Mistral, OpenRouter};
use Step::{Summarize, Transcribe};

// No model for `openai-compatible`: the endpoint behind it is unknown, and models change too fast to guess.
pub const MODELS: &[Entry] = &[
    entry(
        Mistral,
        Transcribe,
        "voxtral-mini-transcribe-3",
        "model.voxtral_mini_transcribe_3",
        true,
        None,
    ),
    entry(
        Mistral,
        Summarize,
        "mistral-small-2603",
        "model.mistral_small_2603",
        true,
        None,
    ),
    // Through OpenRouter, billed on your own Mistral key when routed to `mistral` (BYOK).
    entry(
        OpenRouter,
        Transcribe,
        "mistralai/voxtral-mini-transcribe-3-0",
        "model.voxtral_mini_transcribe_3",
        true,
        None,
    ),
    entry(Claude, Summarize, "sonnet", "model.cli_sonnet", true, None),
    entry(Claude, Summarize, "opus", "model.cli_opus", false, None),
    entry(Claude, Summarize, "haiku", "model.cli_haiku", false, None),
];

/// Curated entries for a provider and step.
pub fn curated(provider: Provider, step: Step) -> impl Iterator<Item = &'static Entry> {
    MODELS
        .iter()
        .filter(move |e| e.provider == provider && e.step == step)
}

/// Whether a model id returned by a provider can do `step`.
pub fn usable(id: &str, step: Step) -> bool {
    let has = |words: &[&str]| words.iter().any(|w| id.contains(w));
    match step {
        // `voxtral` also names text-to-speech and realtime models.
        Step::Transcribe => {
            has(&["transcribe", "whisper", "voxtral"]) && !has(&["tts", "realtime"])
        }
        Step::Summarize => !has(&[
            "embed",
            "moderation",
            "transcribe",
            "whisper",
            "voxtral",
            "tts",
            "image",
            "realtime",
        ]),
    }
}
