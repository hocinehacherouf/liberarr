//! Manual live probe of the AI adapters. Reads only process environment variables and prints one
//! line per case: booleans, counts and the adapters' already-sanitized errors. Never text, audio,
//! summaries, keys or URLs.
//!
//! Variables (all optional, a target with missing ones is skipped): MISTRAL_API_KEY,
//! PROBE_CLAUDE_CLI (model alias for the local `claude` CLI, for example `sonnet`), PROBE_AUDIO
//! (audio file, needed for transcription).
use liberarr_core::Result;
use liberarr_core::lang::Language;
use liberarr_core::summaries::{ChatMessage, Llm, Role, parse_summary, summary_messages};
use liberarr_core::transcription::{ChunkResult, Segment, Transcriber, Transcript};
use liberarr_infra::ai::OpenAiCompat;
use liberarr_infra::claude_cli::ClaudeCli;
use liberarr_infra::mistral::MistralTranscriber;

const MISTRAL_BASE: &str = "https://api.mistral.ai/v1";
const MISTRAL_MODELS: [&str; 4] = [
    "voxtral-mini-latest",
    "voxtral-mini-transcribe-3",
    "voxtral-mini-transcribe-3-0",
    "voxtral-mini-2602",
];

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn show_transcript(label: &str, r: Result<ChunkResult>) {
    match r {
        Ok(c) => println!(
            "{label}: ok text={} segments={} language={}",
            if c.text.trim().is_empty() {
                "no"
            } else {
                "yes"
            },
            c.segments.len(),
            c.language.as_deref().unwrap_or("none")
        ),
        Err(e) => println!("{label}: error {e}"),
    }
}

/// Asks for a tiny JSON object and reports only whether the reply parses.
async fn show_chat(label: &str, llm: &impl Llm) {
    let ask = [ChatMessage {
        role: Role::User,
        content: r#"Reply with the JSON object {"ok":true} and nothing else."#.into(),
    }];
    match llm.complete_json(&ask).await {
        Ok(text) => {
            let valid = serde_json::from_str::<serde_json::Value>(&text).is_ok();
            println!(
                "{label}: ok json={}",
                if valid { "valid" } else { "invalid" }
            );
        }
        Err(e) => println!("{label}: error {e}"),
    }
}

/// Runs the real summary prompt on a synthetic French/English transcript and reports only whether
/// the reply passes the summary validator.
async fn show_summary(label: &str, llm: &impl Llm) {
    let lines = [
        "Bonjour à tous, on fait le point sur la release de vendredi.",
        "Alice: the CI pipeline is green, we just need the security review.",
        "Bob: je m'occupe de la review avant jeudi.",
        "Alice: great, and I will update the changelog.",
    ];
    let segments: Vec<Segment> = lines
        .iter()
        .enumerate()
        .map(|(i, t)| Segment {
            start: i as f64 * 10.0,
            end: i as f64 * 10.0 + 9.0,
            text: (*t).into(),
        })
        .collect();
    let transcript = Transcript {
        engine: "probe".into(),
        language: None,
        text: lines.join(" "),
        segments,
        has_timestamps: true,
    };
    match llm
        .complete_json(&summary_messages(&transcript, Language::En))
        .await
    {
        Ok(text) => match parse_summary(&text) {
            Ok(s) => println!("{label}: ok summary=valid actions={}", s.actions.len()),
            Err(e) => println!("{label}: ok summary=invalid ({e})"),
        },
        Err(e) => println!("{label}: error {e}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let audio = match var("PROBE_AUDIO") {
        Some(path) => Some(std::fs::read(path)?),
        None => None,
    };
    let filename = "probe.ogg";

    match (var("MISTRAL_API_KEY"), &audio) {
        (Some(key), Some(audio)) => {
            for m in MISTRAL_MODELS {
                let t = MistralTranscriber::new(MISTRAL_BASE.into(), key.clone(), m.into());
                show_transcript(&format!("mistral/{m}"), t.transcribe(audio, filename).await);
            }
        }
        _ => println!("mistral transcription: skipped"),
    }
    match var("MISTRAL_API_KEY") {
        Some(key) => {
            let llm = OpenAiCompat::new(MISTRAL_BASE.into(), key, "mistral-small-2603".into());
            show_chat("mistral/chat", &llm).await;
        }
        None => println!("mistral chat: skipped"),
    }

    match var("PROBE_CLAUDE_CLI") {
        Some(model) => {
            let llm = ClaudeCli::new(model);
            show_chat("claude-cli/chat", &llm).await;
            show_summary("claude-cli/summary", &llm).await;
        }
        None => println!("claude cli: skipped"),
    }
    Ok(())
}
