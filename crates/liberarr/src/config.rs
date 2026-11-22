//! `<LIBERARR_HOME>/.env` is the one source of secrets and settings (only `LIBERARR_HOME`, which
//! locates it, comes from the environment). Errors name the variable, never its value.
use liberarr_core::ai::{Provider, Step};
use liberarr_core::lang::Language;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const PLACEHOLDER: &str = "change-me";
/// Minimum length of `LIBERARR_SECRET_KEY` and `LIBERARR_API_KEY`.
const MIN_SECRET: usize = 32;

pub struct Profile {
    pub provider: Provider,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// OpenRouter only: the provider slugs a request is restricted to
    /// (`LIBERARR_{TRANSCRIBE,LLM}_ROUTING_ONLY`).
    /// Empty: no restriction (and for every other provider).
    pub routing_only: Vec<String>,
    /// CLI providers, summary only (`LIBERARR_LLM_PROGRAM`): absolute path of the executable.
    /// `None`: the CLI's name, resolved by the OS PATH.
    pub program: Option<PathBuf>,
}

pub struct Config {
    pub home: PathBuf,
    pub secret_key: String,
    pub api_key: String,
    pub admin_password: String,
    pub transcribe: Profile,
    /// The summary engine is optional: `None` without a summary, the note is exported anyway.
    pub llm: Option<Profile>,
    /// `LIBERARR_TRANSCRIBE_VOCABULARY`: names and terms to bias the transcription toward.
    pub vocabulary: Vec<String>,
    pub obsidian: PathBuf,
    /// Language of the summary and of the note headings.
    pub content_language: Language,
    /// `LIBERARR_CONTENT_LANGUAGE` is in the `.env`: otherwise the language follows the UI's.
    pub content_language_set: bool,
    /// `CLAUDE_CODE_OAUTH_TOKEN` from the `.env`: the `claude` CLI's login where its keychain is out of reach.
    /// Passed to that CLI only, never logged.
    pub claude_oauth_token: Option<String>,
    /// Set the `Secure` cookie flag (serving behind HTTPS).
    pub cookie_secure: bool,
}

/// `LIBERARR_HOME`, default `~/.liberarr`.
pub fn home() -> Result<PathBuf, String> {
    if let Some(h) = std::env::var_os("LIBERARR_HOME").filter(|h| !h.is_empty()) {
        return Ok(h.into());
    }
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".liberarr"))
        .ok_or_else(|| t!("home.not_set").into_owned())
}

/// `KEY=VALUE` lines; blank lines and `#` comments are skipped, surrounding quotes removed.
pub fn parse_env(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.starts_with('#') {
                return None;
            }
            let (k, v) = l.split_once('=')?;
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(v);
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

/// A comma-separated list of OpenRouter provider slugs: trimmed, non-empty, made of lowercase
/// letters, digits, `.`, `_`, `-` and `/` (a variant, never leading or trailing).
pub fn parse_routing(v: &str) -> Option<Vec<String>> {
    let list: Vec<String> = v.split(',').map(|s| s.trim().to_string()).collect();
    list.iter()
        .all(|s| {
            s.chars()
                .any(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && !s.starts_with('/')
                && !s.ends_with('/')
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "./-_".contains(c))
        })
        .then_some(list)
}

/// Where the notes go when `LIBERARR_OBSIDIAN_DIR` is not set.
pub fn default_notes_dir(home: &Path) -> PathBuf {
    home.join("obsidian")
}

pub fn is_unset(v: &str) -> bool {
    let v = v.trim();
    v.is_empty() || v == PLACEHOLDER
}

pub fn load(home: &Path) -> Result<Config, String> {
    let file = match std::fs::read_to_string(home.join(".env")) {
        Ok(t) => parse_env(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(t!("config.env_missing", path = home.join(".env").display()).into_owned());
        }
        Err(e) => return Err(t!("config.env_unreadable", kind = e.kind()).into_owned()),
    };
    let ui = crate::i18n::language(file.get("LIBERARR_LANG").map(String::as_str));
    load_from(home, ui, |k| file.get(k).cloned())
}

fn load_from(
    home: &Path,
    ui: Language,
    get: impl Fn(&str) -> Option<String>,
) -> Result<Config, String> {
    let need = |k: &str| match get(k) {
        Some(v) if !is_unset(&v) => Ok(v.trim().to_string()),
        _ => Err(t!("config.var_unset", var = k, home = home.display()).into_owned()),
    };
    let secret = |k: &str| {
        let v = need(k)?;
        if v.chars().count() < MIN_SECRET {
            return Err(t!("config.secret_too_short", var = k, min = MIN_SECRET).into_owned());
        }
        Ok(v)
    };
    let profile = |p: &str, step: Step| -> Result<Profile, String> {
        let var = format!("LIBERARR_{p}_PROVIDER");
        let provider = match get(&var).filter(|v| !v.trim().is_empty()) {
            None => Provider::Compatible,
            Some(v) => Provider::parse(v.trim()).ok_or_else(|| match v.trim() {
                // The `anthropic` adapter is gone: say what to use instead.
                "anthropic" => t!(
                    "config.provider_removed",
                    var = var,
                    alternatives = match step {
                        Step::Transcribe => "mistral, openrouter",
                        Step::Summarize => "claude, codex, copilot, openrouter",
                    }
                )
                .into_owned(),
                _ => t!("config.provider_unknown", var = var).into_owned(),
            })?,
        };
        // A CLI picks its own default model when none is set.
        let model = if provider.is_cli() {
            get(&format!("LIBERARR_{p}_MODEL"))
                .filter(|v| !is_unset(v))
                .map_or_else(String::new, |v| v.trim().to_string())
        } else {
            need(&format!("LIBERARR_{p}_MODEL"))?
        };
        if !provider.can(step) {
            return Err(match step {
                Step::Transcribe => t!("config.cannot_transcribe", var = var),
                Step::Summarize => t!("config.cannot_summarize", var = var),
            }
            .into_owned());
        }
        // A CLI uses its own login: no URL, no key.
        let (base_url, api_key) = if provider.is_cli() {
            (String::new(), String::new())
        } else {
            (
                need(&format!("LIBERARR_{p}_BASE_URL"))?,
                need(&format!("LIBERARR_{p}_API_KEY"))?,
            )
        };
        // Optional: without it, OpenRouter may serve the request with its own shared models.
        let var = format!("LIBERARR_{p}_ROUTING_ONLY");
        let routing_only = match get(&var).filter(|v| !v.trim().is_empty()) {
            Some(v) if provider == Provider::OpenRouter => parse_routing(&v)
                .ok_or_else(|| t!("config.routing_invalid", var = var).into_owned())?,
            _ => Vec::new(),
        };
        let var = "LIBERARR_LLM_PROGRAM";
        let program = match get(var).filter(|v| !v.trim().is_empty()) {
            Some(v) if step == Step::Summarize && provider.is_cli() => {
                let path = PathBuf::from(v.trim());
                if !path.is_absolute() {
                    return Err(t!("config.program_not_absolute", var = var).into_owned());
                }
                Some(path)
            }
            _ => None,
        };
        Ok(Profile {
            provider,
            base_url,
            api_key,
            model,
            routing_only,
            program,
        })
    };
    // `none`, or nothing at all (provider, URL and model), turns the summary off.
    let summary_engine = !get("LIBERARR_LLM_PROVIDER").is_some_and(|v| v.trim() == "none")
        && ["PROVIDER", "BASE_URL", "MODEL"]
            .iter()
            .any(|k| get(&format!("LIBERARR_LLM_{k}")).is_some_and(|v| !is_unset(&v)));
    let configured = get("LIBERARR_CONTENT_LANGUAGE").filter(|v| !v.trim().is_empty());
    let content_language_set = configured.is_some();
    let content_language = match configured {
        None => ui,
        Some(v) => Language::parse(v.trim())
            .ok_or_else(|| t!("config.content_language_unknown").into_owned())?,
    };
    Ok(Config {
        home: home.to_path_buf(),
        secret_key: secret("LIBERARR_SECRET_KEY")?,
        api_key: secret("LIBERARR_API_KEY")?,
        admin_password: secret("LIBERARR_ADMIN_PASSWORD")?,
        transcribe: profile("TRANSCRIBE", Step::Transcribe)?,
        llm: summary_engine
            .then(|| profile("LLM", Step::Summarize))
            .transpose()?,
        vocabulary: get("LIBERARR_TRANSCRIBE_VOCABULARY")
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        obsidian: get("LIBERARR_OBSIDIAN_DIR")
            .filter(|v| !is_unset(v))
            .map_or_else(|| default_notes_dir(home), PathBuf::from),
        content_language,
        content_language_set,
        claude_oauth_token: get("CLAUDE_CODE_OAUTH_TOKEN")
            .filter(|v| !is_unset(v))
            .map(|v| v.trim().to_string()),
        cookie_secure: get("LIBERARR_COOKIE_SECURE").is_some_and(|v| v.trim() == "true"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "LIBERARR_SECRET_KEY=0123456789abcdef0123456789abcdef
LIBERARR_API_KEY=fedcba9876543210fedcba9876543210
LIBERARR_ADMIN_PASSWORD=pw
LIBERARR_TRANSCRIBE_BASE_URL=https://api.openai.com/v1
LIBERARR_TRANSCRIBE_API_KEY=sk-a
LIBERARR_TRANSCRIBE_MODEL=whisper-1
LIBERARR_LLM_BASE_URL=https://api.openai.com/v1
LIBERARR_LLM_API_KEY=sk-b
LIBERARR_LLM_MODEL=gpt
";

    fn load_text(text: &str) -> Result<Config, String> {
        let mut vars = parse_env(text);
        // Long enough to pass, built here so secret scanners do not flag the fixture.
        vars.entry("LIBERARR_ADMIN_PASSWORD".into())
            .and_modify(|v| {
                if v == "pw" {
                    *v = "pw".repeat(16);
                }
            });
        load_from(Path::new("/h"), Language::En, |k| vars.get(k).cloned())
    }

    fn with(key: &str, value: &str) -> String {
        VALID
            .lines()
            .map(|l| {
                if l.starts_with(&format!("{key}=")) {
                    format!("{key}={value}")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn config_refuses_placeholder_without_echoing() {
        assert!(load_text(VALID).is_ok());
        for (key, value) in [
            ("LIBERARR_SECRET_KEY", "change-me"),
            ("LIBERARR_SECRET_KEY", ""),
            (
                "LIBERARR_SECRET_KEY",
                "short-secret-value-31-chars-xxxx"[..31].as_ref(),
            ),
            ("LIBERARR_API_KEY", "\"change-me\""),
            ("LIBERARR_API_KEY", "only-twenty-chars-ab"),
            ("LIBERARR_LLM_API_KEY", "change-me"),
            ("LIBERARR_ADMIN_PASSWORD", "hunter2"),
        ] {
            let msg = load_text(&with(key, value))
                .err()
                .unwrap_or_else(|| panic!("{key}={value} accepted"));
            assert!(msg.contains(key), "{msg}");
            assert!(
                value.is_empty() || !msg.contains(value.trim_matches('"')),
                "{msg}"
            );
        }
        // A missing line is refused like an empty one.
        let missing: String = VALID
            .lines()
            .filter(|l| !l.starts_with("LIBERARR_SECRET_KEY"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            load_text(&missing)
                .err()
                .is_some_and(|m| m.contains("LIBERARR_SECRET_KEY"))
        );
    }

    fn provider_line(key: &str, value: &str) -> String {
        format!("{VALID}{key}={value}\n")
    }

    #[test]
    fn openrouter_routing_list_is_optional_but_validated() {
        let or = |routing: Option<&str>| {
            let mut t = provider_line("LIBERARR_LLM_PROVIDER", "openrouter");
            if let Some(r) = routing {
                t.push_str(&format!("LIBERARR_LLM_ROUTING_ONLY={r}\n"));
            }
            load_text(&t)
        };
        assert_eq!(
            or(Some("azure")).ok().unwrap().llm.unwrap().routing_only,
            ["azure"]
        );
        for none in [None, Some("")] {
            assert!(or(none).ok().unwrap().llm.unwrap().routing_only.is_empty());
        }
        for bad in ["azure,", "Azure!", "-"] {
            let msg = or(Some(bad)).err().unwrap();
            assert!(msg.contains("LIBERARR_LLM_ROUTING_ONLY"), "{msg}");
            // `-` is also in the hint text: only longer values prove the value is not echoed.
            assert!(
                (bad.len() < 2 || !msg.contains(bad)) && !msg.contains("sk-b"),
                "{msg}"
            );
        }
    }

    #[test]
    fn cli_provider_cannot_be_the_transcriber() {
        let msg = load_text(&provider_line("LIBERARR_TRANSCRIBE_PROVIDER", "claude"))
            .err()
            .unwrap();
        assert!(msg.contains("LIBERARR_TRANSCRIBE_PROVIDER"), "{msg}");
        assert!(!msg.contains("sk-a"), "{msg}");
    }

    #[test]
    fn cli_provider_needs_no_model_and_an_absolute_program() {
        let cli = |extra: &str| {
            load_text(&format!(
                "{}LIBERARR_LLM_PROVIDER=codex\n{extra}",
                VALID.replace("LIBERARR_LLM_MODEL=gpt\n", "")
            ))
        };
        let llm = cli("").unwrap().llm.unwrap();
        assert_eq!((llm.provider, llm.model.as_str()), (Provider::Codex, ""));
        assert!(llm.program.is_none());
        let llm = cli("LIBERARR_LLM_PROGRAM=/opt/codex\n")
            .unwrap()
            .llm
            .unwrap();
        assert_eq!(llm.program, Some(PathBuf::from("/opt/codex")));
        let msg = cli("LIBERARR_LLM_PROGRAM=codex\n").err().unwrap();
        assert!(msg.contains("LIBERARR_LLM_PROGRAM"), "{msg}");
    }

    #[test]
    fn removed_and_aliased_providers() {
        // `anthropic` is refused by name, with the key kept out of the message.
        let msg = load_text(&provider_line("LIBERARR_LLM_PROVIDER", "anthropic"))
            .err()
            .unwrap();
        assert!(msg.contains("LIBERARR_LLM_PROVIDER"), "{msg}");
        assert!(!msg.contains("sk-b"), "{msg}");
        // `openai` still works, as an alias of `openai-compatible`.
        let cfg = load_text(&provider_line("LIBERARR_LLM_PROVIDER", "openai")).unwrap();
        assert_eq!(cfg.llm.unwrap().provider, Provider::Compatible);
    }

    #[test]
    fn summary_engine_is_optional() {
        let llm_lines = |keep: bool| -> String {
            VALID
                .lines()
                .filter(|l| {
                    keep || !["BASE_URL", "MODEL"]
                        .iter()
                        .any(|k| l.contains(&format!("LLM_{k}")))
                })
                .map(|l| format!("{l}\n"))
                .collect()
        };
        // Nothing about the summary: off.
        assert!(load_text(&llm_lines(false)).unwrap().llm.is_none());
        // `none` wins over the other LLM lines.
        let off = load_text(&provider_line("LIBERARR_LLM_PROVIDER", "none")).unwrap();
        assert!(off.llm.is_none());
        // A URL and a model without a provider stay an openai-compatible gateway.
        let llm = load_text(&llm_lines(true)).unwrap().llm.unwrap();
        assert_eq!(llm.provider, Provider::Compatible);
        // `none` is for the summary only.
        assert!(load_text(&provider_line("LIBERARR_TRANSCRIBE_PROVIDER", "none")).is_err());
    }
}
