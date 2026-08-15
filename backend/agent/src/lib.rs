use std::{path::PathBuf, process::Stdio, sync::Arc};

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::{
    io::AsyncWriteExt,
    process::Command,
    sync::Mutex,
};

#[derive(Clone)]
pub struct CodexAgent {
    state: Arc<Mutex<SessionState>>,
}

struct SessionState {
    session_id: String,
    working_directory: PathBuf,
}

impl CodexAgent {
    pub async fn new(initial_prompt: impl Into<String>) -> Result<Self> {
        let working_directory = std::env::current_dir().context("failed to get working directory")?;
        let turn = run_codex(
            &["exec", "--json", "--sandbox", "read-only", "--skip-git-repo-check", "-"],
            &initial_prompt.into(),
            &working_directory,
        )
        .await
        .context("failed to start Codex session")?;

        Ok(Self {
            state: Arc::new(Mutex::new(SessionState {
                session_id: turn
                    .session_id
                    .context("Codex did not return a session ID")?,
                working_directory,
            })),
        })
    }

    pub fn prompt<F>(&self, prompt: String, callback: F)
    where
        F: FnOnce(String) + Send + 'static,
    {
        let state = Arc::clone(&self.state);
        tokio::spawn(async move {
            let result = async {
                let state = state.lock().await;
                let turn = run_codex(
                    &["exec", "resume", "--json", &state.session_id, "-"],
                    &prompt,
                    &state.working_directory,
                )
                .await
                .context("failed to prompt Codex session")?;
                let response = turn
                    .response
                    .context("Codex did not return an agent response")?;
                callback(response);
                Result::<()>::Ok(())
            }
            .await;

            if let Err(error) = result {
                eprintln!("Codex agent failed: {error:#}");
            }
        });
    }
}

struct CodexTurn {
    session_id: Option<String>,
    response: Option<String>,
}

async fn run_codex(args: &[&str], prompt: &str, working_directory: &PathBuf) -> Result<CodexTurn> {
    let mut child = Command::new("codex")
        .args(args)
        .current_dir(working_directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn codex CLI")?;

    child
        .stdin
        .take()
        .context("failed to open codex stdin")?
        .write_all(prompt.as_bytes())
        .await
        .context("failed to write prompt to codex CLI")?;

    let output = child
        .wait_with_output()
        .await
        .context("failed to wait for codex CLI")?;
    if !output.status.success() {
        bail!(
            "codex CLI exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    parse_codex_output(&String::from_utf8(output.stdout).context("codex output was not UTF-8")?)
}

fn parse_codex_output(output: &str) -> Result<CodexTurn> {
    let mut turn = CodexTurn {
        session_id: None,
        response: None,
    };

    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let event: Value = serde_json::from_str(line).context("invalid JSON event from codex CLI")?;
        match event.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                turn.session_id = event
                    .get("thread_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            Some("item.completed")
                if event.pointer("/item/type").and_then(Value::as_str)
                    == Some("agent_message") =>
            {
                turn.response = event
                    .pointer("/item/text")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
            }
            _ => {}
        }
    }

    Ok(turn)
}

#[cfg(test)]
mod tests {
    use super::parse_codex_output;

    #[test]
    fn parses_session_and_agent_response() {
        let output = concat!(
            r#"{"type":"thread.started","thread_id":"session-123"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"Welcome!"}}"#,
        );

        let turn = parse_codex_output(output).unwrap();
        assert_eq!(turn.session_id.as_deref(), Some("session-123"));
        assert_eq!(turn.response.as_deref(), Some("Welcome!"));
    }
}
