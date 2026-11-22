//! The `liberarr init` flow, written against [`Prompter`] only: a front-end (plain CLI today, a
//! terminal UI later) implements the trait and nothing here knows how a question is asked.
//! Secrets are never passed to the prompter except as the answer to a `secret` question, and are
//! never printed.
mod check;
mod noninteractive;
mod plain;
#[cfg(test)]
mod tests;

pub use check::{connect, routes};
pub use noninteractive::from_env;
pub use plain::PlainPrompter;

use crate::config::{is_unset, parse_routing};
use crate::{models, providers};
use liberarr_core::ai::{Provider, Step};
use liberarr_core::lang::Language;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub type SetupResult<T> = Result<T, Box<dyn std::error::Error>>;
/// `(variable, value)` lines to write in the `.env`.
pub type Updates = Vec<(&'static str, String)>;
/// Tests what the account can do: for an API provider, the models it lists; for `claude`, whether
/// the CLI is signed in (empty list). The error text never carries the key.
pub type Checker<'a> = dyn Fn(Provider, &str, &str) -> Result<Vec<String>, String> + 'a;
/// The providers that serve an OpenRouter model `(base URL, key, model)`, as routing slugs.
pub type Routes<'a> = dyn Fn(&str, &str, &str) -> Result<Vec<String>, String> + 'a;

/// One line of a menu: the value that is returned (and can be typed) and a short note.
pub struct Choice {
    pub value: String,
    pub note: String,
}

pub enum Picked {
    Option(usize),
    /// Free text that matches no option; a front-end without free text never returns it.
    Text(String),
}

pub trait Prompter {
    /// False when no one can answer (piped stdin): the keep/reconfigure question is then skipped.
    fn is_interactive(&self) -> bool;
    fn choose(
        &mut self,
        question: &str,
        options: &[Choice],
        default: Option<usize>,
    ) -> std::io::Result<Picked>;
    /// Never returns an empty string: it uses `default`, or asks again.
    fn text(&mut self, question: &str, default: Option<&str>) -> std::io::Result<String>;
    /// Hidden input, never empty.
    fn secret(&mut self, question: &str) -> std::io::Result<String>;
    fn confirm(&mut self, question: &str, default: bool) -> std::io::Result<bool>;
    fn info(&mut self, line: &str);
    fn warn(&mut self, line: &str);
    /// Runs `work` while showing that something is going on.
    fn busy(&mut self, label: &str, work: &mut dyn FnMut());
}

/// What to write, and the admin password when it was just generated (shown once).
pub struct Plan {
    pub updates: Updates,
    pub password: Option<String>,
}

/// Per step: the step, the variables (provider, base URL, API key, model) and the questions
/// (provider, base URL, API key, model): catalog keys.
type StepVars = (Step, [&'static str; 4], [&'static str; 4]);

const STEPS: [StepVars; 2] = [
    (
        Step::Transcribe,
        [
            "LIBERARR_TRANSCRIBE_PROVIDER",
            "LIBERARR_TRANSCRIBE_BASE_URL",
            "LIBERARR_TRANSCRIBE_API_KEY",
            "LIBERARR_TRANSCRIBE_MODEL",
        ],
        [
            "init.transcribe.provider",
            "init.transcribe.url",
            "init.transcribe.key",
            "init.transcribe.model",
        ],
    ),
    (
        Step::Summarize,
        [
            "LIBERARR_LLM_PROVIDER",
            "LIBERARR_LLM_BASE_URL",
            "LIBERARR_LLM_API_KEY",
            "LIBERARR_LLM_MODEL",
        ],
        [
            "init.summarize.provider",
            "init.summarize.url",
            "init.summarize.key",
            "init.summarize.model",
        ],
    ),
];

/// A key is retyped at most this many times before the user must confirm to go on.
const KEY_TRIES: usize = 3;
/// Models the account returns beyond the curated ones: a menu of 100 lines is not a menu.
const MAX_OTHER_MODELS: usize = 15;

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    /// Asks only what is missing: a set value is never overwritten.
    Fill,
    /// Asks the providers again, with the current values as defaults.
    Reconfigure,
}

/// `http(s)://host[...]`: what the HTTP client accepts as a base URL, without a URL parser.
pub(crate) fn valid_url(u: &str) -> bool {
    let Some(rest) = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !host.is_empty() && !host.starts_with(':') && !u.chars().any(char::is_whitespace)
}

/// Asks for a base URL until it is valid, at most [`KEY_TRIES`] times.
fn ask_url(p: &mut dyn Prompter, question: &str, default: Option<&str>) -> SetupResult<String> {
    for _ in 0..KEY_TRIES {
        let url = p.text(question, default)?;
        if valid_url(&url) {
            return Ok(url);
        }
        p.warn(&t!("setup.url_invalid"));
    }
    Err(t!("setup.aborted").into_owned().into())
}

/// The OpenRouter provider list of a step.
pub(crate) fn routing_var(step: Step) -> &'static str {
    match step {
        Step::Transcribe => "LIBERARR_TRANSCRIBE_ROUTING_ONLY",
        Step::Summarize => "LIBERARR_LLM_ROUTING_ONLY",
    }
}

/// Asks for the optional OpenRouter provider list until it is valid, at most [`KEY_TRIES`] times:
/// a menu of the providers serving the model when known (a list can still be typed), else a
/// question. Returns the normalized list, empty for no restriction (`-`). With no current list,
/// the menu has no default: restricting or not is an explicit choice.
fn ask_routing(
    p: &mut dyn Prompter,
    var: &str,
    model: &str,
    offered: &[String],
    current: Option<&str>,
) -> SetupResult<String> {
    let mut values: Vec<String> = offered.to_vec();
    if let Some(c) = current.filter(|c| !values.iter().any(|v| v == c)) {
        values.insert(0, c.to_string());
    }
    let mut choices: Vec<Choice> = values
        .iter()
        .map(|v| Choice {
            value: v.clone(),
            note: if Some(v.as_str()) == current {
                t!("setup.current").into_owned()
            } else {
                String::new()
            },
        })
        .collect();
    choices.push(Choice {
        value: "-".into(),
        note: t!("setup.routing_none").into_owned(),
    });
    for _ in 0..KEY_TRIES {
        let v = if offered.is_empty() {
            p.text(&t!("setup.routing"), Some(current.unwrap_or("-")))?
        } else {
            let default = current.map(|c| values.iter().position(|v| v == c).unwrap_or(0));
            match p.choose(&t!("setup.routing_menu", model = model), &choices, default)? {
                Picked::Option(i) => choices[i].value.clone(),
                Picked::Text(t) => t,
            }
        };
        if v.trim() == "-" {
            return Ok(String::new());
        }
        if let Some(list) = parse_routing(&v) {
            return Ok(list.join(","));
        }
        p.warn(&t!("config.routing_invalid", var = var));
    }
    Err(t!("setup.aborted").into_owned().into())
}

/// The value of `key` unless missing, empty or a placeholder.
pub(crate) fn current(env: &HashMap<String, String>, key: &str) -> Option<String> {
    env.get(key)
        .map(|v| v.trim())
        .filter(|v| !is_unset(v))
        .map(str::to_string)
}

fn random_hex(bytes: usize) -> SetupResult<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|_| t!("init.random_unavailable").into_owned())?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Generates the secrets that are missing. Only here: a reconfigure never reaches this for a set
/// secret, and `LIBERARR_SECRET_KEY` encrypts the stored device keys, so it must never change.
pub(crate) fn secrets(env: &HashMap<String, String>) -> SetupResult<Plan> {
    let mut updates = Updates::new();
    for k in ["LIBERARR_SECRET_KEY", "LIBERARR_API_KEY"] {
        if current(env, k).is_none() {
            updates.push((k, random_hex(32)?));
        }
    }
    let password = if current(env, "LIBERARR_ADMIN_PASSWORD").is_none() {
        let p = random_hex(16)?;
        updates.push(("LIBERARR_ADMIN_PASSWORD", p.clone()));
        Some(p)
    } else {
        None
    };
    Ok(Plan { updates, password })
}

/// What the `.env` says about the provider of a step.
#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Current {
    /// Not chosen yet, or a value that is not a provider (`anthropic`, a typo): ask again.
    Unset,
    /// No provider for this step, by choice. Not produced yet.
    #[allow(dead_code)]
    Off,
    Is(Provider),
}

/// The provider already chosen for a step. A base URL without a provider is a gateway.
pub(crate) fn current_provider(
    env: &HashMap<String, String>,
    provider_var: &str,
    url_var: &str,
) -> Current {
    match current(env, provider_var) {
        Some(v) => Provider::parse(&v).map_or(Current::Unset, Current::Is),
        None => current(env, url_var).map_or(Current::Unset, |_| Current::Is(Provider::Compatible)),
    }
}

/// The provider line holds a value that is not a provider: what was stored for the step (URL,
/// key, model, routing) belonged to something else and is ignored.
pub(crate) fn provider_invalid(env: &HashMap<String, String>, provider_var: &str) -> bool {
    current(env, provider_var).is_some_and(|v| Provider::parse(&v).is_none())
}

/// Secrets to generate, then the missing provider settings. When everything is already set and
/// someone can answer, offers to keep it or to reconfigure the providers.
pub fn run_setup(
    p: &mut dyn Prompter,
    env: &HashMap<String, String>,
    ui: Language,
    home: &Path,
    check: &Checker,
    routes: &Routes,
) -> SetupResult<Plan> {
    let mut plan = secrets(env)?;
    plan.updates
        .extend(configure(p, env, Mode::Fill, ui, home, check, routes)?);
    if plan.updates.is_empty() && p.is_interactive() {
        let options = [
            Choice {
                value: t!("setup.keep").into_owned(),
                note: String::new(),
            },
            Choice {
                value: t!("setup.reconfigure").into_owned(),
                note: String::new(),
            },
        ];
        if let Picked::Option(1) = p.choose(&t!("setup.existing"), &options, Some(0))? {
            plan.updates = configure(p, env, Mode::Reconfigure, ui, home, check, routes)?;
        }
    }
    Ok(plan)
}

/// Pushes `value` unless the `.env` already has exactly that.
fn set(updates: &mut Updates, env: &HashMap<String, String>, key: &'static str, value: String) {
    if current(env, key).as_deref() != Some(value.as_str()) {
        updates.push((key, value));
    }
}

fn configure(
    p: &mut dyn Prompter,
    env: &HashMap<String, String>,
    mode: Mode,
    ui: Language,
    home: &Path,
    check: &Checker,
    routes: &Routes,
) -> SetupResult<Updates> {
    let reconf = mode == Mode::Reconfigure;
    let mut updates = Updates::new();
    // `claude auth status` is run at most once.
    let mut claude_ok: Option<bool> = None;
    let mut claude_ready =
        || *claude_ok.get_or_insert_with(|| check(Provider::Claude, "", "").is_ok());
    for (step, [provider_var, url_var, key_var, model_var], [provider_q, url_q, key_q, model_q]) in
        STEPS
    {
        let known = match current_provider(env, provider_var, url_var) {
            Current::Is(p) => Some(p),
            Current::Unset | Current::Off => None,
        };
        let provider = match known.filter(|_| !reconf) {
            Some(known) => known,
            None => {
                if step == Step::Summarize && claude_ready() {
                    p.info(&t!("init.claude_detected"));
                }
                let chosen = ask_provider(p, &t!(provider_q), step, known)?;
                set(&mut updates, env, provider_var, chosen.as_str().to_string());
                chosen
            }
        };
        // Another provider: the old URL, key and model do not apply.
        let changed = (reconf && known != Some(provider)) || provider_invalid(env, provider_var);
        let (default_url, default_model) = providers::defaults(provider, step);
        let own = |k: &str| current(env, k).filter(|_| !changed);

        // The claude CLI has its own login: no URL, no key.
        let needs_api = !provider.is_cli();
        let mut url = own(url_var).unwrap_or_default();
        if needs_api && (reconf || url.is_empty()) {
            let default = Some(url.as_str()).filter(|u| !u.is_empty()).or(default_url);
            url = ask_url(p, &t!(url_q), default)?;
            set(&mut updates, env, url_var, url.clone());
        }

        let model_asked = reconf || changed || current(env, model_var).is_none();
        let mut found = Vec::new();
        let mut key = own(key_var).unwrap_or_default();
        if needs_api {
            let ask_key =
                key.is_empty() || (reconf && p.confirm(&t!("setup.change_key"), false)?);
            if ask_key {
                let (typed, models) = ask_key_tested(p, &t!(key_q), provider, &url, check)?;
                key = typed.clone();
                set(&mut updates, env, key_var, typed);
                found = models;
            } else if model_asked {
                // The stored key still feeds the model list; a failure only costs the live list.
                found = test(p, check, provider, &url, &key).unwrap_or_else(|why| {
                    p.warn(&t!("setup.check_failed", why = why));
                    Vec::new()
                });
            }
        } else if model_asked && !claude_ready() {
            p.warn(&t!("setup.claude_not_ready"));
        }

        let model = if model_asked {
            // OpenRouter lists hundreds of chat models: the question gives an example instead.
            let model_q = match (provider, step) {
                (Provider::OpenRouter, Step::Summarize) => "init.summarize.model_openrouter",
                _ => model_q,
            };
            let model = ask_model(
                p,
                &t!(model_q),
                provider,
                step,
                &found,
                own(model_var).as_deref().or(default_model),
                own(model_var).is_some(),
            )?;
            set(&mut updates, env, model_var, model.clone());
            model
        } else {
            current(env, model_var).unwrap_or_default()
        };

        // Asked after the model: the menu lists the providers that serve it. Optional, so an
        // empty answer is written (`KEY=`): its absence means "not asked yet".
        let var = routing_var(step);
        if provider == Provider::OpenRouter && (reconf || changed || !env.contains_key(var)) {
            let mut offered = Ok(Vec::new());
            p.busy(&t!("setup.routing_loading"), &mut || {
                offered = routes(&url, &key, &model);
            });
            // Unknown model or no network: the question without a menu still works.
            let offered = offered.unwrap_or_default();
            let routing = ask_routing(p, var, &model, &offered, own(var).as_deref())?;
            if routing.is_empty() {
                p.warn(&t!("init.openrouter_unrestricted"));
            } else {
                p.info(&t!("init.openrouter_shared_capacity"));
            }
            if env.get(var).map(|v| v.trim()) != Some(routing.as_str()) {
                updates.push((var, routing));
            }
        }
    }
    // Asked once: left unset, the notes would change language with the OS locale.
    if reconf || current(env, CONTENT_LANGUAGE).is_none() {
        let known = current(env, CONTENT_LANGUAGE).and_then(|v| Language::parse(&v));
        let chosen = ask_content_language(p, ui, known)?;
        set(
            &mut updates,
            env,
            CONTENT_LANGUAGE,
            chosen.as_str().to_string(),
        );
    }
    if reconf || current(env, NOTES_DIR).is_none() {
        let default = current(env, NOTES_DIR)
            .unwrap_or_else(|| crate::config::default_notes_dir(home).display().to_string());
        let dir = ask_notes_dir(p, &default)?;
        set(&mut updates, env, NOTES_DIR, dir);
    }
    Ok(updates)
}

pub(crate) const CONTENT_LANGUAGE: &str = "LIBERARR_CONTENT_LANGUAGE";
pub(crate) const NOTES_DIR: &str = "LIBERARR_OBSIDIAN_DIR";

/// The notes folder as written in the `.env`: `~/` expanded, absolute (a relative path would
/// follow the working directory), created when missing. The error is the message to show.
pub(crate) fn notes_dir(v: &str) -> Result<String, String> {
    let v = v.trim();
    let path = match (v.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(h)) => PathBuf::from(h).join(rest),
        _ => PathBuf::from(v),
    };
    if !path.is_absolute() || v.chars().any(char::is_control) {
        return Err(t!("setup.notes_dir_absolute").into_owned());
    }
    std::fs::create_dir_all(&path).map_err(|e| {
        t!(
            "setup.notes_dir_unusable",
            path = path.display(),
            kind = e.kind()
        )
        .into_owned()
    })?;
    Ok(path.display().to_string())
}

/// Asks for the notes folder until it is usable, at most [`KEY_TRIES`] times.
fn ask_notes_dir(p: &mut dyn Prompter, default: &str) -> SetupResult<String> {
    for _ in 0..KEY_TRIES {
        let v = match p.text(&t!("setup.notes_dir"), Some(default)) {
            // Closed stdin (a provisioning script): no one can answer, take the default.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => default.to_string(),
            r => r?,
        };
        match notes_dir(&v) {
            Ok(dir) => return Ok(dir),
            Err(why) => p.warn(&why),
        }
    }
    Err(t!("setup.aborted").into_owned().into())
}

/// Language of the summaries and the note headings; defaults to the current value, else the UI's.
fn ask_content_language(
    p: &mut dyn Prompter,
    ui: Language,
    known: Option<Language>,
) -> SetupResult<Language> {
    let all = [Language::En, Language::Fr];
    let choices: Vec<Choice> = all
        .iter()
        .map(|l| Choice {
            value: l.as_str().to_string(),
            note: String::new(),
        })
        .collect();
    let default = all.iter().position(|l| *l == known.unwrap_or(ui));
    loop {
        let picked = match p.choose(&t!("setup.content_language"), &choices, default) {
            // Closed stdin (a provisioning script): no one can answer, take the default.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(known.unwrap_or(ui));
            }
            r => r?,
        };
        match picked {
            Picked::Option(i) => return Ok(all[i]),
            Picked::Text(t) => match Language::parse(&t) {
                Some(l) => return Ok(l),
                None => p.warn(&t!("setup.language_invalid")),
            },
        }
    }
}

fn ask_provider(
    p: &mut dyn Prompter,
    question: &str,
    step: Step,
    known: Option<Provider>,
) -> SetupResult<Provider> {
    let options: Vec<Provider> = [
        Provider::Compatible,
        Provider::Mistral,
        Provider::OpenRouter,
        Provider::Claude,
    ]
    .into_iter()
    .filter(|o| o.can(step))
    .collect();
    let choices: Vec<Choice> = options
        .iter()
        .map(|o| Choice {
            value: o.as_str().to_string(),
            note: String::new(),
        })
        .collect();
    let default = options.iter().position(|o| Some(*o) == known).unwrap_or(0);
    loop {
        match p.choose(question, &choices, Some(default))? {
            Picked::Option(i) => return Ok(options[i]),
            Picked::Text(_) => p.warn(&t!("init.provider_unknown")),
        }
    }
}

/// Asks for the key, then tests it. A failure offers to retype it; after [`KEY_TRIES`] or a
/// refusal to retype, going on with the key needs an explicit yes.
fn ask_key_tested(
    p: &mut dyn Prompter,
    question: &str,
    provider: Provider,
    url: &str,
    check: &Checker,
) -> SetupResult<(String, Vec<String>)> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let key = p.secret(question)?;
        // The count, never the content: a pasted key that is too short shows at once.
        p.info(&t!("setup.secret_received", count = key.chars().count()));
        match test(p, check, provider, url, &key) {
            Ok(models) => return Ok((key, models)),
            Err(why) => {
                p.warn(&t!("setup.check_failed", why = why));
                if attempt < KEY_TRIES && p.confirm(&t!("setup.retype_key"), true)? {
                    continue;
                }
                if p.confirm(&t!("setup.continue_anyway"), false)? {
                    return Ok((key, Vec::new()));
                }
                return Err(t!("setup.aborted").into_owned().into());
            }
        }
    }
}

fn test(
    p: &mut dyn Prompter,
    check: &Checker,
    provider: Provider,
    url: &str,
    key: &str,
) -> Result<Vec<String>, String> {
    let mut out = None;
    p.busy(&t!("setup.testing"), &mut || {
        out = Some(check(provider, url, key));
    });
    let models = out.unwrap_or_else(|| Ok(Vec::new()))?;
    p.info(&match models.len() {
        0 => t!("setup.check_ok").into_owned(),
        n => t!("setup.check_ok_models", count = n).into_owned(),
    });
    Ok(models)
}

/// `value, note` of a curated entry: its label, whether it is recommended, its retirement date.
fn curated_note(e: &models::Entry) -> String {
    let mut parts = vec![t!(e.label).into_owned()];
    if e.recommended {
        parts.push(t!("setup.recommended").into_owned());
    }
    if let Some(date) = e.deprecated {
        parts.push(t!("setup.deprecated", date = date).into_owned());
    }
    parts.join(", ")
}

/// Menu of models: the curated ones (all when the account's list is unknown, else the recommended
/// ones it has), then the other usable models it lists, then "type a name". `preferred` is the
/// default; a `current` model missing from the menu is added so a reconfigure can keep it.
fn ask_model(
    p: &mut dyn Prompter,
    question: &str,
    provider: Provider,
    step: Step,
    found: &[String],
    preferred: Option<&str>,
    is_current: bool,
) -> SetupResult<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    if found.is_empty() {
        entries
            .extend(models::curated(provider, step).map(|e| (e.id.to_string(), curated_note(e))));
    } else {
        entries.extend(
            models::curated(provider, step)
                .filter(|e| e.recommended && found.iter().any(|f| f == e.id))
                .map(|e| (e.id.to_string(), curated_note(e))),
        );
        let mut others: Vec<&String> = found
            .iter()
            .filter(|id| models::usable(id, step) && !entries.iter().any(|(v, _)| v == *id))
            .collect();
        others.sort();
        others.dedup();
        entries.extend(others.into_iter().take(MAX_OTHER_MODELS).map(|id| {
            let note = models::curated(provider, step)
                .find(|e| e.id == id)
                .map(curated_note)
                .unwrap_or_default();
            (id.clone(), note)
        }));
    }
    if let Some(m) = preferred.filter(|m| is_current && !entries.iter().any(|(v, _)| v == m)) {
        entries.insert(0, (m.to_string(), t!("setup.current").into_owned()));
    }
    let default = preferred
        .and_then(|m| entries.iter().position(|(v, _)| v == m))
        .or((!entries.is_empty()).then_some(0));
    let mut choices: Vec<Choice> = entries
        .iter()
        .map(|(value, note)| Choice {
            value: value.clone(),
            note: note.clone(),
        })
        .collect();
    choices.push(Choice {
        value: t!("setup.type_name").into_owned(),
        note: String::new(),
    });
    let model = match p.choose(question, &choices, default)? {
        Picked::Option(i) if i < entries.len() => entries.swap_remove(i).0,
        Picked::Option(_) => p.text(question, preferred)?,
        Picked::Text(name) => name,
    };
    // Still accepted: the list can be partial. A wrong name fails the job later.
    if !found.is_empty() && !found.contains(&model) {
        p.warn(&t!("setup.model_unlisted"));
    }
    Ok(model)
}

#[cfg(test)]
mod url_tests {
    use super::valid_url;

    #[test]
    fn base_url_needs_scheme_and_host() {
        for ok in [
            "https://api.mistral.ai/v1",
            "http://127.0.0.1:8080",
            "http://h?x",
        ] {
            assert!(valid_url(ok), "{ok}");
        }
        for bad in [
            "mistral",
            "ftp://h",
            "https://",
            "http:///v1",
            "http://:80",
            "http://a b",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
    }
}
