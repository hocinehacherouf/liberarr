//! Summaries through the user's logged-in `claude` CLI, run isolated (no tools, no settings, no hooks); no token is ever read.
use liberarr_core::summaries::{ChatMessage, Llm, Role};
use liberarr_core::{Error, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(300);

pub struct ClaudeCli {
    program: String,
    model: String,
}

impl ClaudeCli {
    pub fn new(model: String) -> Self {
        Self::with_program("claude".into(), model)
    }

    pub(crate) fn with_program(program: String, model: String) -> Self {
        Self { program, model }
    }

    /// Blocking: runs the CLI with `prompt` on stdin and returns its stdout. Errors carry fixed text only.
    fn run(&self, system: Option<String>, prompt: String) -> Result<Vec<u8>> {
        let err = |m: &str| Error::Provider(m.into());
        let mut cmd = Command::new(&self.program);
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
            "--model",
        ])
        .arg(&self.model);
        if let Some(s) = system {
            cmd.arg("--system-prompt").arg(s);
        }
        // Use the login, not API billing.
        let mut child = cmd
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("ANTHROPIC_AUTH_TOKEN")
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                err(if e.kind() == std::io::ErrorKind::NotFound {
                    "claude CLI not found in PATH"
                } else {
                    "claude CLI could not be started"
                })
            })?;
        // Threads keep the pipes flowing so a large prompt or reply cannot block the child.
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| err("claude CLI could not be started"))?;
        std::thread::spawn(move || {
            let _ = stdin.write_all(prompt.as_bytes());
        });
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| err("claude CLI could not be started"))?;
        let reader = std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            out
        });
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(s)) => break s,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(err("claude CLI timed out"));
                }
                Err(_) => return Err(err("claude CLI could not be started")),
            }
        };
        if !status.success() {
            return Err(Error::Provider(format!(
                "claude CLI exited with status {}: check that `claude` is up to date and logged in",
                status.code().unwrap_or(-1)
            )));
        }
        reader
            .join()
            .map_err(|_| err("claude CLI could not be started"))
    }
}

impl Llm for ClaudeCli {
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
        let prompt = rest
            .iter()
            .map(|m| match (labelled, m.role) {
                (false, _) => m.content.clone(),
                (true, Role::Assistant) => format!("Assistant: {}", m.content),
                (true, _) => format!("User: {}", m.content),
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let (program, model) = (self.program.clone(), self.model.clone());
        let out = tokio::task::spawn_blocking(move || {
            ClaudeCli::with_program(program, model).run(system, prompt)
        })
        .await
        .map_err(|_| Error::Provider("claude CLI could not be started".into()))??;
        let invalid = || Error::Provider("claude CLI reply is not valid".into());
        let v: Value = serde_json::from_slice(&out).map_err(|_| invalid())?;
        if v.get("is_error").and_then(Value::as_bool) == Some(true) {
            return Err(Error::Provider("claude CLI reported an error".into()));
        }
        v.get("result")
            .and_then(Value::as_str)
            .map(String::from)
            .ok_or_else(invalid)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn prompt_goes_through_stdin_not_argv() {
        const MARKER: &str = "MARKER-7f3a";
        let dir = std::env::temp_dir().join(format!("liberarr-claude-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("claude");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncat > '{d}/stdin'\nprintf '%s\\n' \"$@\" > '{d}/args'\n\
                 echo '{{\"result\":\"{{\\\"a\\\":1}}\",\"is_error\":false}}'\n",
                d = dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let cli = ClaudeCli::with_program(script.to_string_lossy().into(), "sonnet".into());
        let out = cli
            .complete_json(&[
                ChatMessage {
                    role: Role::System,
                    content: "be brief".into(),
                },
                ChatMessage {
                    role: Role::User,
                    content: format!("hello {MARKER}"),
                },
            ])
            .await;
        let stdin = std::fs::read_to_string(dir.join("stdin")).unwrap();
        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(out.unwrap(), r#"{"a":1}"#);
        assert!(stdin.contains(MARKER));
        assert!(!args.contains(MARKER));
        assert!(args.contains("--no-session-persistence"));
    }
}
