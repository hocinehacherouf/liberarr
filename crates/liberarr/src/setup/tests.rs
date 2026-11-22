use super::*;
use crate::commands::init::merge;
use crate::config::parse_env;
use std::collections::VecDeque;

const ENV: &str = "# my notes
LIBERARR_SECRET_KEY=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
LIBERARR_API_KEY=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
LIBERARR_ADMIN_PASSWORD=admin-pw-secret
LIBERARR_TRANSCRIBE_PROVIDER=mistral
LIBERARR_TRANSCRIBE_BASE_URL=https://api.mistral.ai/v1
LIBERARR_TRANSCRIBE_API_KEY=stored-transcribe-key
LIBERARR_TRANSCRIBE_MODEL=voxtral-mini-transcribe-3
LIBERARR_LLM_PROVIDER=mistral
LIBERARR_LLM_BASE_URL=https://api.mistral.ai/v1
LIBERARR_LLM_API_KEY=stored-llm-key
LIBERARR_LLM_MODEL=mistral-small-2603
LIBERARR_CONTENT_LANGUAGE=fr
LIBERARR_OBSIDIAN_DIR=/tmp
";
const SECRETS: [&str; 3] = [
    "LIBERARR_SECRET_KEY",
    "LIBERARR_API_KEY",
    "LIBERARR_ADMIN_PASSWORD",
];
const TYPED_KEY: &str = "typed-new-key-123";

/// Answers the questions with scripted values and records everything it is shown.
#[derive(Default)]
struct Fake {
    choices: VecDeque<Picked>,
    secrets: VecDeque<&'static str>,
    confirms: VecDeque<bool>,
    /// `choose` fails as a closed stdin does.
    closed: bool,
    seen: Vec<String>,
}

impl Prompter for Fake {
    fn is_interactive(&self) -> bool {
        true
    }
    fn choose(&mut self, q: &str, o: &[Choice], _: Option<usize>) -> std::io::Result<Picked> {
        self.seen.push(q.to_string());
        self.seen
            .extend(o.iter().map(|c| format!("{} {}", c.value, c.note)));
        if self.closed {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        Ok(self.choices.pop_front().expect("unexpected choice"))
    }
    fn text(&mut self, q: &str, d: Option<&str>) -> std::io::Result<String> {
        self.seen.push(format!("{q} {d:?}"));
        Ok(d.expect("unexpected text").to_string())
    }
    fn secret(&mut self, q: &str) -> std::io::Result<String> {
        self.seen.push(q.to_string());
        Ok(self.secrets.pop_front().expect("unexpected secret").into())
    }
    fn confirm(&mut self, q: &str, _: bool) -> std::io::Result<bool> {
        self.seen.push(q.to_string());
        Ok(self.confirms.pop_front().expect("unexpected confirm"))
    }
    fn info(&mut self, line: &str) {
        self.seen.push(line.to_string());
    }
    fn warn(&mut self, line: &str) {
        self.seen.push(line.to_string());
    }
    fn busy(&mut self, _: &str, work: &mut dyn FnMut()) {
        work();
    }
}

fn ok(_: Provider, _: &str, _: &str) -> Result<Vec<String>, String> {
    Ok(vec![])
}

fn no_routes(_: &str, _: &str, _: &str) -> Result<Vec<String>, String> {
    Err("offline".into())
}

fn written(plan: &Plan) -> HashMap<String, String> {
    parse_env(&merge(ENV, &plan.updates))
}

#[test]
fn set_values_are_never_overwritten_or_re_asked() {
    // A .env missing only the LLM model: nothing else may be asked or changed.
    let partial = ENV.replace("LIBERARR_LLM_MODEL=mistral-small-2603\n", "");
    let env = parse_env(&partial);
    let mut p = Fake::default();
    p.choices.push_back(Picked::Text("my-model".into()));
    let plan = run_setup(&mut p, &env, Language::En, Path::new("/h"), &ok, &no_routes).unwrap();
    assert_eq!(
        plan.updates,
        [("LIBERARR_LLM_MODEL", "my-model".to_string())]
    );
    assert!(plan.password.is_none());
}

#[test]
fn reconfigure_keeps_the_secrets_and_never_shows_a_secret() {
    let env = parse_env(ENV);
    let mut p = Fake::default();
    // Keep or reconfigure: reconfigure. Transcription: mistral again (index 1 of 3), key kept,
    // first model. Summary: switch to openai-compatible (index 0), which forces a new key; a
    // custom model.
    p.choices.extend([
        Picked::Option(1),
        Picked::Option(1),
        Picked::Option(0),
        Picked::Option(0),
        Picked::Text("gpt-custom".into()),
        Picked::Option(0),
    ]);
    p.confirms.push_back(false);
    p.secrets.push_back(TYPED_KEY);
    let plan = run_setup(&mut p, &env, Language::En, Path::new("/h"), &ok, &no_routes).unwrap();

    let out = written(&plan);
    for k in SECRETS {
        assert_eq!(out[k], parse_env(ENV)[k], "{k} changed");
        assert!(plan.updates.iter().all(|(u, _)| *u != k), "{k} rewritten");
    }
    assert_eq!(out["LIBERARR_TRANSCRIBE_API_KEY"], "stored-transcribe-key");
    assert_eq!(out["LIBERARR_LLM_PROVIDER"], "openai-compatible");
    assert_eq!(out["LIBERARR_LLM_API_KEY"], TYPED_KEY);
    assert_eq!(out["LIBERARR_LLM_MODEL"], "gpt-custom");
    assert_eq!(out["LIBERARR_CONTENT_LANGUAGE"], "en");
    assert!(plan.password.is_none());

    // Nothing the front-end was shown contains a secret, stored or typed.
    let seen = p.seen.join("\n");
    for secret in [
        TYPED_KEY,
        "stored-transcribe-key",
        "stored-llm-key",
        "admin-pw-secret",
        &env["LIBERARR_SECRET_KEY"],
        &env["LIBERARR_API_KEY"],
    ] {
        assert!(!seen.contains(secret), "{secret} reached the front-end");
    }
}

/// A provider that no longer exists (`anthropic`) is not read as a gateway: its URL, key and
/// model are never kept, in either mode.
#[test]
fn a_removed_provider_is_asked_again_with_its_settings_dropped() {
    let stale = ENV
        .replace(
            "LIBERARR_LLM_PROVIDER=mistral",
            "LIBERARR_LLM_PROVIDER=anthropic",
        )
        .replace(
            "https://api.mistral.ai/v1\nLIBERARR_LLM_API_KEY",
            "https://api.anthropic.com\nLIBERARR_LLM_API_KEY",
        )
        .replace(
            "LIBERARR_LLM_MODEL=mistral-small-2603",
            "LIBERARR_LLM_MODEL=claude-sonnet-5-5",
        );
    let env = parse_env(&stale);
    assert_eq!(env["LIBERARR_LLM_BASE_URL"], "https://api.anthropic.com");

    // Interactive: the summary provider (mistral, index 1), then its URL, key and model.
    let mut p = Fake::default();
    p.choices.extend([Picked::Option(1), Picked::Option(0)]);
    p.secrets.push_back(TYPED_KEY);
    let plan = run_setup(&mut p, &env, Language::En, Path::new("/h"), &ok, &no_routes).unwrap();
    assert!(
        p.seen
            .iter()
            .any(|s| s.contains(&*t!("init.summarize.provider")))
    );
    let out = parse_env(&merge(&stale, &plan.updates));
    assert_eq!(out["LIBERARR_LLM_PROVIDER"], "mistral");
    assert_eq!(out["LIBERARR_LLM_BASE_URL"], "https://api.mistral.ai/v1");
    assert_eq!(out["LIBERARR_LLM_API_KEY"], TYPED_KEY);
    assert_eq!(out["LIBERARR_LLM_MODEL"], "mistral-small-2603");
    assert!(plan.updates.iter().all(|(_, v)| !v.contains("anthropic")));

    // Non-interactive: read again from the environment, the stored values ignored.
    let given = |k: &str| match k {
        "LIBERARR_LLM_PROVIDER" => Some("mistral".to_string()),
        "LIBERARR_LLM_API_KEY" => Some(TYPED_KEY.to_string()),
        "LIBERARR_LLM_MODEL" => Some("mistral-small-2603".to_string()),
        _ => None,
    };
    let plan = from_env(&env, Language::En, Path::new("/h"), given).unwrap();
    let out = parse_env(&merge(&stale, &plan.updates));
    assert_eq!(out["LIBERARR_LLM_PROVIDER"], "mistral");
    assert_eq!(out["LIBERARR_LLM_BASE_URL"], "https://api.mistral.ai/v1");
    assert_eq!(out["LIBERARR_LLM_API_KEY"], TYPED_KEY);
    assert_eq!(out["LIBERARR_LLM_MODEL"], "mistral-small-2603");
}

#[test]
fn closed_stdin_takes_the_default_content_language() {
    let env = parse_env(&ENV.replace("LIBERARR_CONTENT_LANGUAGE=fr\n", ""));
    let mut p = Fake {
        closed: true,
        ..Fake::default()
    };
    let plan = run_setup(&mut p, &env, Language::En, Path::new("/h"), &ok, &no_routes).unwrap();
    assert_eq!(
        plan.updates,
        [("LIBERARR_CONTENT_LANGUAGE", "en".to_string())]
    );
}

/// Money guard: an OpenRouter step gets its own provider list, picked from the providers that
/// serve the chosen model, and the other step's list is left alone.
#[test]
fn openrouter_routing_is_asked_per_step_from_the_model_providers() {
    let env = parse_env(
        &ENV.replace(
            "LIBERARR_TRANSCRIBE_PROVIDER=mistral",
            "LIBERARR_TRANSCRIBE_PROVIDER=openrouter",
        )
        .replace(
            "LIBERARR_TRANSCRIBE_MODEL=voxtral-mini-transcribe-3",
            "LIBERARR_TRANSCRIBE_MODEL=mistralai/voxtral-mini-transcribe-3-0",
        ),
    );
    let routes = |_: &str, key: &str, model: &str| {
        assert_eq!(key, "stored-transcribe-key");
        assert_eq!(model, "mistralai/voxtral-mini-transcribe-3-0");
        Ok(vec!["mistral".to_string()])
    };
    let mut p = Fake::default();
    p.choices.push_back(Picked::Option(0));
    let plan = run_setup(&mut p, &env, Language::En, Path::new("/h"), &ok, &routes).unwrap();
    assert_eq!(
        plan.updates,
        [("LIBERARR_TRANSCRIBE_ROUTING_ONLY", "mistral".to_string())]
    );
}

/// The non-interactive mode writes the notes folder it was given, created, and refuses a path
/// that would follow the working directory.
#[test]
fn non_interactive_writes_a_usable_notes_folder() {
    let env = parse_env(&ENV.replace("LIBERARR_OBSIDIAN_DIR=/tmp\n", ""));
    let dir = std::env::temp_dir().join(format!("liberarr-notes-{}", std::process::id()));
    let given = dir.display().to_string();
    let plan = from_env(&env, Language::En, Path::new("/h"), |k| {
        (k == "LIBERARR_OBSIDIAN_DIR").then(|| given.clone())
    })
    .unwrap();
    assert_eq!(plan.updates, [("LIBERARR_OBSIDIAN_DIR", given.clone())]);
    assert!(dir.is_dir());
    std::fs::remove_dir(&dir).unwrap();

    let relative = from_env(&env, Language::En, Path::new("/h"), |k| {
        (k == "LIBERARR_OBSIDIAN_DIR").then(|| "notes".to_string())
    });
    assert!(relative.is_err());
}

/// The summary is optional: `none` is written when nothing is given (non-interactive), is read
/// back as a choice, and is never asked about again. Transcription stays required.
#[test]
fn the_summary_engine_can_be_left_out() {
    let without = ENV
        .lines()
        .filter(|l| !l.starts_with("LIBERARR_LLM_"))
        .collect::<Vec<_>>()
        .join("\n");
    let env = parse_env(&without);
    let plan = from_env(&env, Language::En, Path::new("/h"), |_| None).unwrap();
    assert_eq!(
        plan.updates,
        [("LIBERARR_LLM_PROVIDER", "none".to_string())]
    );
    // A URL given without a provider is not silently dropped.
    let url = |k: &str| (k == "LIBERARR_LLM_BASE_URL").then(|| "https://gw.example/v1".to_string());
    assert!(from_env(&env, Language::En, Path::new("/h"), url).is_err());
    // Nothing for transcription: still refused.
    let none = parse_env(
        &ENV.replace("LIBERARR_TRANSCRIBE_PROVIDER=mistral\n", "")
            .replace(
                "LIBERARR_TRANSCRIBE_BASE_URL=https://api.mistral.ai/v1\n",
                "",
            ),
    );
    assert!(from_env(&none, Language::En, Path::new("/h"), |_| None).is_err());

    let off = ENV
        .replace(
            "LIBERARR_LLM_PROVIDER=mistral",
            "LIBERARR_LLM_PROVIDER=none",
        )
        .replace("LIBERARR_LLM_API_KEY=stored-llm-key\n", "")
        .replace("LIBERARR_LLM_BASE_URL=https://api.mistral.ai/v1\n", "")
        .replace("LIBERARR_LLM_MODEL=mistral-small-2603\n", "");
    let env = parse_env(&off);
    assert_eq!(
        current_provider(
            &env,
            Step::Summarize,
            "LIBERARR_LLM_PROVIDER",
            "LIBERARR_LLM_BASE_URL"
        ),
        Current::Off
    );
    // Interactive: only the keep-or-reconfigure question, nothing about the summary.
    let mut p = Fake::default();
    p.choices.push_back(Picked::Option(0));
    let plan = run_setup(&mut p, &env, Language::En, Path::new("/h"), &ok, &no_routes).unwrap();
    assert!(plan.updates.is_empty());
    assert!(
        !p.seen
            .iter()
            .any(|s| s.contains(&*t!("init.summarize.provider")))
    );
    // Non-interactive: nothing to add, nothing fails.
    let plan = from_env(&env, Language::En, Path::new("/h"), |_| None).unwrap();
    assert!(plan.updates.is_empty());

    // Reconfigure the summary from a provider to `none` (the last entry, index 4).
    let env = parse_env(ENV);
    let mut p = Fake::default();
    // Reconfigure, transcription (mistral, first model), summary (none), content language.
    p.choices.extend([
        Picked::Option(1),
        Picked::Option(1),
        Picked::Option(0),
        Picked::Option(4),
        Picked::Option(0),
    ]);
    p.confirms.push_back(false);
    let plan = run_setup(&mut p, &env, Language::En, Path::new("/h"), &ok, &no_routes).unwrap();
    assert_eq!(written(&plan)["LIBERARR_LLM_PROVIDER"], "none");
}
