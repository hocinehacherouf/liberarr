//! Summaries through the user's own logged-in CLI (claude, codex, copilot). The transcript is
//! untrusted text: each CLI runs with its tools locked down, in a fresh empty directory, and may
//! only answer.
use liberarr_core::ai::Provider;
use liberarr_core::summaries::{ChatMessage, Llm, Role};
use liberarr_core::{Error, Result};
use serde_json::Value;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(600);
/// A reply is a summary: more than this is a runaway, never read into memory.
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    Claude,
    Codex,
    Copilot,
}

impl Preset {
    pub fn from_provider(p: Provider) -> Option<Preset> {
        match p {
            Provider::Claude => Some(Preset::Claude),
            Provider::Codex => Some(Preset::Codex),
            Provider::Copilot => Some(Preset::Copilot),
            _ => None,
        }
    }

    /// The executable name: "claude", "codex", "copilot".
    pub fn program_name(self) -> &'static str {
        match self {
            Preset::Claude => "claude",
            Preset::Codex => "codex",
            Preset::Copilot => "copilot",
        }
    }

    /// Variables removed so the CLI uses the user's login, not API billing or a looser mode.
    fn env_removed(self) -> &'static [&'static str] {
        match self {
            Preset::Claude => &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"],
            Preset::Copilot => &["COPILOT_ALLOW_ALL"],
            Preset::Codex => &["OPENAI_API_KEY", "CODEX_API_KEY"],
        }
    }
}

#[derive(Clone)]
pub struct AgentCli {
    preset: Preset,
    program: PathBuf,
    model: String,
    extra_env: Vec<(String, String)>,
    timeout: Duration,
}

impl AgentCli {
    /// `program`: absolute path from LIBERARR_LLM_PROGRAM, else the preset's name resolved by the OS PATH.
    /// `model`: empty = the CLI's default (no model flag). `claude_oauth_token`: the claude CLI's
    /// login where its keychain is out of reach; the other CLIs never get it.
    pub fn new(
        preset: Preset,
        program: Option<PathBuf>,
        model: String,
        claude_oauth_token: Option<String>,
    ) -> Self {
        let extra_env = claude_oauth_token
            .filter(|_| preset == Preset::Claude)
            .map(|t| ("CLAUDE_CODE_OAUTH_TOKEN".to_string(), t))
            .into_iter()
            .collect();
        Self {
            preset,
            program: program.unwrap_or_else(|| preset.program_name().into()),
            model,
            extra_env,
            timeout: TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// The fully configured command. The prompt is never an argument: it goes on stdin.
    /// `system` is a flag for Claude; the other presets get it in front of the prompt.
    pub(crate) fn command(&self, system: Option<&str>, dir: &Path) -> Command {
        let mut cmd = Command::new(&self.program);
        let model = (!self.model.is_empty()).then_some(self.model.as_str());
        match self.preset {
            Preset::Claude => {
                cmd.args([
                    "-p",
                    "--output-format",
                    "json",
                    "--no-session-persistence",
                    "--tools",
                    "",
                    "--setting-sources",
                    "",
                    "--disable-slash-commands",
                    "--strict-mcp-config",
                ]);
                if let Some(m) = model {
                    cmd.args(["--model", m]);
                }
                if let Some(s) = system {
                    cmd.args(["--system-prompt", s]);
                }
            }
            // No -p: with it, stdin is ignored.
            Preset::Copilot => {
                cmd.args([
                    "--output-format",
                    "json",
                    "-s",
                    "--deny-tool",
                    "shell",
                    "--deny-tool",
                    "write",
                    "--deny-tool",
                    "url",
                    "--disable-builtin-mcps",
                    "--disallow-temp-dir",
                    "--no-ask-user",
                    "--no-auto-update",
                    // The session copy under ~/.copilot/session-state is kept anyway: no flag turns it off.
                    "--sync-session=off",
                    "--no-custom-instructions",
                ]);
                if let Some(m) = model {
                    cmd.args(["--model", m]);
                }
            }
            Preset::Codex => {
                cmd.args([
                    "exec",
                    "--skip-git-repo-check",
                    "--sandbox",
                    "read-only",
                    "--color",
                    "never",
                    "--json",
                    // Nothing from the user's config.toml (MCP servers, hooks, rules) and no session file.
                    "--ignore-user-config",
                    "--ignore-rules",
                    "--ephemeral",
                    "-c",
                    "features.shell_tool=false",
                    // Keeps the user's own hooks from running on the transcript.
                    "-c",
                    "features.hooks=false",
                    "-c",
                    "web_search=\"disabled\"",
                ]);
                for feature in [
                    "apps",
                    "plugins",
                    "browser_use",
                    "computer_use",
                    "view_image",
                    "multi_agent",
                    "image_generation",
                ] {
                    cmd.args(["--disable", feature]);
                }
                if let Some(m) = model {
                    cmd.args(["-m", m]);
                }
                cmd.arg("-");
            }
        }
        for k in self.preset.env_removed() {
            cmd.env_remove(k);
        }
        cmd.envs(self.extra_env.iter().map(|(k, v)| (k, v)))
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        cmd
    }

    /// Blocking: runs the CLI in a fresh empty directory with `prompt` on stdin and returns its
    /// stdout. Errors carry fixed text only.
    fn run(&self, system: Option<&str>, prompt: String) -> Result<Vec<u8>> {
        let name = self.preset.program_name();
        let err = |m: &str| Error::Provider(format!("{name} CLI {m}"));
        let dir = ScratchDir::new().map_err(|_| err("could not be started"))?;
        let mut child = KillOnDrop(self.command(system, &dir.0).spawn().map_err(|e| {
            err(if e.kind() == std::io::ErrorKind::NotFound {
                "not found (check PATH or LIBERARR_LLM_PROGRAM)"
            } else {
                "could not be started"
            })
        })?);
        // Threads keep the pipes flowing so a large prompt or reply cannot block the child.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| err("could not be started"))?;
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| err("could not be started"))?;
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.take(MAX_OUTPUT + 1).read_to_end(&mut out);
            out
        });
        let (mut reader, mut status, mut out) = (Some(reader), None, None);
        let deadline = Instant::now() + self.timeout;
        let (status, out) = loop {
            if status.is_none() {
                status = child.try_wait().map_err(|_| err("could not be started"))?;
            }
            if let Some(r) = reader.take_if(|r| r.is_finished()) {
                let o = r.join().map_err(|_| err("could not be started"))?;
                if o.len() as u64 > MAX_OUTPUT {
                    return Err(err("output too large"));
                }
                out = Some(o);
            }
            match (status, out.take()) {
                (Some(s), Some(o)) => break (s, o),
                (_, o) => out = o,
            }
            if Instant::now() >= deadline {
                // The reader is not joined: a process the CLI started can hold the pipe open.
                // shortcut: processes started by the CLI itself (node-based CLIs) can outlive the kill; upgrade to a process group kill if it bites.
                return Err(err("timed out"));
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if !status.success() {
            return Err(Error::Provider(format!(
                "{name} CLI exited with status {}: check that `{name}` is up to date and logged in",
                status.code().unwrap_or(-1)
            )));
        }
        Ok(out)
    }

    /// The reply text of `out`, or None when it is not the shape the preset promises.
    fn reply(&self, out: &[u8]) -> Option<String> {
        let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(String::from);
        let lines = || {
            out.split(|b| *b == b'\n')
                .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        };
        match self.preset {
            Preset::Claude => {
                let v: Value = serde_json::from_slice(out).ok()?;
                if v["is_error"] == true {
                    return None;
                }
                text(&v["result"])
            }
            Preset::Copilot => lines()
                .filter(|v| v["type"] == "assistant.message")
                .filter_map(|v| text(&v["data"]["content"]))
                .next_back(),
            Preset::Codex => lines()
                .filter(|v| v["type"] == "item.completed" && v["item"]["type"] == "agent_message")
                .filter_map(|v| text(&v["item"]["text"]))
                .next_back(),
        }
    }
}

/// Kills and reaps the child when dropped, so every early return of `run` stops it.
struct KillOnDrop(std::process::Child);

impl std::ops::Deref for KillOnDrop {
    type Target = std::process::Child;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for KillOnDrop {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A fresh empty directory (mode 0700) for one run, removed when dropped, whatever the outcome.
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> std::io::Result<Self> {
        loop {
            let mut h = RandomState::new().build_hasher();
            h.write_u32(std::process::id());
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            let path = std::env::temp_dir().join(format!("liberarr-cli-{:016x}", h.finish()));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Llm for AgentCli {
    async fn complete_json(&self, messages: &[ChatMessage]) -> Result<String> {
        let (system, rest): (Vec<_>, Vec<_>) =
            messages.iter().partition(|m| m.role == Role::System);
        let system = (!system.is_empty()).then(|| {
            system
                .iter()
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        });
        let labelled = rest.len() > 1;
        let mut prompt = rest
            .iter()
            .map(|m| match (labelled, m.role) {
                (false, _) => m.content.clone(),
                (true, Role::Assistant) => format!("Assistant: {}", m.content),
                (true, _) => format!("User: {}", m.content),
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        if self.preset != Preset::Claude
            && let Some(s) = &system
        {
            prompt = format!("{s}\n\n{prompt}");
        }
        let this = self.clone();
        let name = self.preset.program_name();
        let out = tokio::task::spawn_blocking(move || this.run(system.as_deref(), prompt))
            .await
            .map_err(|_| Error::Provider(format!("{name} CLI could not be started")))??;
        self.reply(&out)
            .ok_or_else(|| Error::Provider(format!("{name} CLI reply is not valid")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    const ALL: [Preset; 3] = [Preset::Claude, Preset::Codex, Preset::Copilot];
    const REPLY: &str = r#"{"a":1}"#;

    /// A valid sample output of each preset, carrying REPLY.
    fn sample(p: Preset) -> &'static str {
        match p {
            Preset::Claude => "{\"result\":\"{\\\"a\\\":1}\",\"is_error\":false}\n",
            Preset::Copilot => {
                "{\"type\":\"session.start\"}\n{\"type\":\"assistant.message\",\"data\":{\"content\":\"{\\\"a\\\":1}\"}}\n{\"type\":\"result\"}\n"
            }
            Preset::Codex => {
                "{\"type\":\"turn.started\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"{\\\"a\\\":1}\"}}\n{\"type\":\"turn.completed\"}\n"
            }
        }
    }

    /// A directory holding a fake CLI named like the preset, with the sample output in a file
    /// named reply, and `body` (a shell script) as the CLI.
    fn fake(p: Preset, reply: &str, body: &str) -> (PathBuf, PathBuf) {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "liberarr-agent-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("reply"), reply).unwrap();
        let script = dir.join(p.program_name());
        let body = body.replace("{d}", &dir.display().to_string());
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, script)
    }

    fn user(text: &str) -> Vec<ChatMessage> {
        vec![
            ChatMessage {
                role: Role::System,
                content: "be brief".into(),
            },
            ChatMessage {
                role: Role::User,
                content: text.into(),
            },
        ]
    }

    fn cli(p: Preset, script: &Path, model: &str) -> AgentCli {
        AgentCli::new(p, Some(script.into()), model.into(), None)
    }

    /// The full argv each preset must get for model "m1" and the system prompt "be brief":
    /// dropping any lockdown flag has to fail the test.
    fn expected_argv(p: Preset) -> Vec<&'static str> {
        match p {
            Preset::Claude => vec![
                "-p",
                "--output-format",
                "json",
                "--no-session-persistence",
                "--tools",
                "",
                "--setting-sources",
                "",
                "--disable-slash-commands",
                "--strict-mcp-config",
                "--model",
                "m1",
                "--system-prompt",
                "be brief",
            ],
            Preset::Copilot => vec![
                "--output-format",
                "json",
                "-s",
                "--deny-tool",
                "shell",
                "--deny-tool",
                "write",
                "--deny-tool",
                "url",
                "--disable-builtin-mcps",
                "--disallow-temp-dir",
                "--no-ask-user",
                "--no-auto-update",
                "--sync-session=off",
                "--no-custom-instructions",
                "--model",
                "m1",
            ],
            Preset::Codex => vec![
                "exec",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                "--color",
                "never",
                "--json",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "-c",
                "features.shell_tool=false",
                "-c",
                "features.hooks=false",
                "-c",
                "web_search=\"disabled\"",
                "--disable",
                "apps",
                "--disable",
                "plugins",
                "--disable",
                "browser_use",
                "--disable",
                "computer_use",
                "--disable",
                "view_image",
                "--disable",
                "multi_agent",
                "--disable",
                "image_generation",
                "-m",
                "m1",
                "-",
            ],
        }
    }

    #[tokio::test]
    async fn prompt_goes_through_stdin_not_argv() {
        const MARKER: &str = "MARKER-7f3a";
        for p in ALL {
            let (dir, script) = fake(
                p,
                sample(p),
                "cat > '{d}/stdin'\nprintf '%s\\n' \"$@\" > '{d}/args'\ncat '{d}/reply'",
            );
            let out = cli(p, &script, "m1")
                .complete_json(&user(&format!("hello {MARKER}")))
                .await;
            let stdin = std::fs::read_to_string(dir.join("stdin")).unwrap();
            let args = std::fs::read_to_string(dir.join("args")).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();

            assert_eq!(out.unwrap(), REPLY, "{p:?}");
            assert!(stdin.contains(MARKER), "{p:?}");
            assert!(!args.contains(MARKER), "{p:?}");
            let lines: Vec<&str> = args.lines().collect();
            assert_eq!(lines, expected_argv(p), "{p:?}");
            match p {
                // The system message is a flag for Claude, part of the prompt elsewhere.
                Preset::Claude => assert!(args.contains("be brief")),
                _ => assert!(stdin.contains("be brief") && !args.contains("be brief")),
            }
        }
    }

    #[tokio::test]
    async fn agent_cli_runs_in_fresh_empty_dir() {
        for p in ALL {
            for (exit, ok) in [(0, true), (1, false)] {
                let (dir, script) = fake(
                    p,
                    sample(p),
                    &format!(
                        "pwd > '{{d}}/pwd'\nls -A > '{{d}}/ls'\ncat > /dev/null\ncat '{{d}}/reply'\nexit {exit}"
                    ),
                );
                let out = cli(p, &script, "").complete_json(&user("x")).await;
                let cwd = std::fs::read_to_string(dir.join("pwd")).unwrap();
                let ls = std::fs::read_to_string(dir.join("ls")).unwrap();
                std::fs::remove_dir_all(&dir).unwrap();

                assert_eq!(out.is_ok(), ok, "{p:?}");
                let cwd = PathBuf::from(cwd.trim());
                assert!(ls.is_empty(), "{p:?}: {ls}");
                assert!(
                    cwd.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("liberarr-cli-"),
                    "{cwd:?}"
                );
                assert!(!cwd.exists(), "{cwd:?} left behind");
            }
        }
    }

    #[tokio::test]
    async fn agent_cli_times_out() {
        let p = Preset::Codex;
        let (dir, script) = fake(p, "", "pwd > '{d}/pwd'\nexec sleep 30");
        let start = Instant::now();
        let err = cli(p, &script, "")
            .with_timeout(Duration::from_secs(1))
            .complete_json(&user("x"))
            .await
            .unwrap_err()
            .to_string();
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(err.contains("codex") && err.contains("timed out"), "{err}");
        let cwd = std::fs::read_to_string(dir.join("pwd")).unwrap();
        assert!(!Path::new(cwd.trim()).exists(), "{cwd} left behind");
        std::fs::remove_dir_all(&dir).unwrap();

        // More than 16 MiB is refused at once, not at the timeout.
        let (dir, big) = fake(p, "", "head -c 20000000 /dev/zero");
        let start = Instant::now();
        let err = cli(p, &big, "")
            .with_timeout(Duration::from_secs(5))
            .complete_json(&user("x"))
            .await
            .unwrap_err()
            .to_string();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(start.elapsed() < Duration::from_secs(4), "{err}");
        assert!(err.contains("codex") && err.contains("too large"), "{err}");
    }

    #[tokio::test]
    async fn agent_cli_rejects_unexpected_output() {
        const SECRET: &str = "SECRET-TEXT";
        for (p, out) in [
            (
                Preset::Claude,
                format!("{{\"is_error\":true,\"result\":\"{SECRET}\"}}"),
            ),
            (
                Preset::Copilot,
                format!(
                    "{{\"type\":\"session.start\"}}\n{{\"type\":\"error\",\"message\":\"{SECRET}\"}}"
                ),
            ),
            (
                Preset::Codex,
                format!("{{\"type\":\"error\",\"message\":\"{SECRET}\"}}"),
            ),
        ] {
            let (dir, script) = fake(p, &out, "cat > /dev/null\ncat '{d}/reply'");
            let err = cli(p, &script, "")
                .complete_json(&user("x"))
                .await
                .unwrap_err()
                .to_string();
            std::fs::remove_dir_all(&dir).unwrap();
            assert!(err.contains(p.program_name()), "{err}");
            assert!(!err.contains(SECRET), "{err}");
        }
    }

    #[test]
    fn only_claude_gets_the_oauth_token() {
        let env = |p: Preset, token: Option<&str>| {
            AgentCli::new(p, None, String::new(), token.map(String::from))
                .command(None, Path::new("/x"))
                .get_envs()
                .map(|(k, v)| {
                    (
                        k.to_string_lossy().into_owned(),
                        v.map(|v| v.to_string_lossy().into_owned()),
                    )
                })
                .collect::<Vec<_>>()
        };
        let claude = env(Preset::Claude, Some("tok-test"));
        for want in [
            ("CLAUDE_CODE_OAUTH_TOKEN", Some("tok-test")),
            ("ANTHROPIC_API_KEY", None),
            ("ANTHROPIC_AUTH_TOKEN", None),
        ] {
            let want = (want.0.to_string(), want.1.map(String::from));
            assert!(claude.contains(&want), "{want:?} not in {claude:?}");
        }
        for p in [Preset::Codex, Preset::Copilot] {
            let e = env(p, Some("tok-test"));
            assert!(
                e.iter().all(|(k, _)| k != "CLAUDE_CODE_OAUTH_TOKEN"),
                "{p:?}: {e:?}"
            );
        }
        let codex = env(Preset::Codex, None);
        assert!(codex.contains(&("OPENAI_API_KEY".into(), None)));
    }
}
