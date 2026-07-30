//! The Claude engine: drives the `claude` CLI headless with stream-json, plus
//! cancellation, the wall-clock timeout, the `SONGSMITH_MOCK_CLAUDE` test hook,
//! and JSON extraction from model output. Split out of `agent.rs` (audit
//! Tier-2 #11) — no DB or stage knowledge lives here.

use crate::models::Settings;
use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

// ---- Cancellation ------------------------------------------------------------

/// Cancels an in-flight Claude call. Clone it freely; `cancel()` releases every
/// current *and future* `cancelled().await` (a pre-fired token resolves
/// immediately, so there is no arm-before-fire race).
#[derive(Clone)]
pub struct CancelToken(std::sync::Arc<tokio::sync::watch::Sender<bool>>);

impl CancelToken {
    pub fn new() -> Self {
        Self(std::sync::Arc::new(tokio::sync::watch::channel(false).0))
    }
    pub fn cancel(&self) {
        // send_replace, not send: `send` is a no-op while no receiver exists yet,
        // which would drop a cancel fired before the call loop starts listening
        let _ = self.0.send_replace(true);
    }
    /// Resolves once `cancel()` has been called (immediately if it already was).
    pub async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        // wait_for checks the current value first; Err (sender dropped) cannot
        // happen while `self` holds the sender.
        let _ = rx.wait_for(|c| *c).await;
    }
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Wall-clock limit for one Claude CLI call: 600s, overridable via the
/// `SONGSMITH_CLAUDE_TIMEOUT_SECS` env var.
fn claude_timeout() -> std::time::Duration {
    let secs = std::env::var("SONGSMITH_CLAUDE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(600);
    std::time::Duration::from_secs(secs)
}

/// Generate text with Claude by driving the `claude` CLI headless with
/// stream-json. Streams text deltas via `on_token`, returns the final answer.
/// Hardened: `kill_on_drop`, concurrent stderr drain (read-after-wait deadlocks
/// past ~64KB), a wall-clock timeout, an optional `cancel` token, and the
/// `result` event's `is_error`/`subtype` treated as failure.
pub(crate) async fn call_claude<F>(settings: &Settings, system: &str, user: &str, on_token: &F, cancel: Option<&CancelToken>) -> Result<String>
where
    F: Fn(String) + Send,
{
    // Test hook: when SONGSMITH_MOCK_CLAUDE is set, return its value verbatim as
    // Claude's output instead of shelling out to the CLI. Lets the freeze/merge
    // logic be exercised end-to-end in `cargo test` with no Claude subscription.
    if let Ok(canned) = std::env::var("SONGSMITH_MOCK_CLAUDE") {
        on_token(canned.clone());
        return Ok(canned);
    }

    let bin = if settings.claude_bin.is_empty() { "claude".to_string() } else { settings.claude_bin.clone() };
    let mut cmd = tokio::process::Command::new(&bin);
    // This app drives Claude via your Claude Code / claude.ai subscription login.
    // If ANTHROPIC_API_KEY (or the helper var) is inherited from the launching
    // environment, the CLI silently switches to API billing and disables connectors.
    // Strip them so it always uses the subscription login.
    cmd.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN");
    // The user prompt goes over STDIN, not as a `-p` argument: prompts can start
    // with `-` (the canonical SECTIONS block opens with "-----"), which the CLI's
    // argument parser rejects as an unknown option (user-hit crash).
    cmd.arg("-p")
        .arg("--append-system-prompt").arg(system)
        .arg("--output-format").arg("stream-json")
        .arg("--verbose")
        .arg("--include-partial-messages")
        .arg("--no-session-persistence")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // no zombie `claude` if this future is dropped (timeout / cancel / panic)
        .kill_on_drop(true);
    if !settings.claude_model.is_empty() {
        cmd.arg("--model").arg(&settings.claude_model);
    }

    let mut child = cmd.spawn().map_err(|e| anyhow!("could not start the claude CLI ({bin}): {e}. Install Claude Code and sign in."))?;
    // write the prompt and close stdin so the CLI sees EOF and starts the turn
    {
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("claude stdin unavailable"))?;
        use tokio::io::AsyncWriteExt;
        stdin.write_all(user.as_bytes()).await?;
        stdin.shutdown().await?;
        drop(stdin);
    }
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("claude stdout unavailable"))?;

    // Drain stderr CONCURRENTLY with the stdout read — reading it only after
    // `wait()` deadlocks once the CLI writes more than the pipe buffer (~64KB).
    let stderr = child.stderr.take();
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        if let Some(mut se) = stderr {
            let _ = se.read_to_string(&mut buf).await;
        }
        buf
    });

    let mut lines = tokio::io::BufReader::new(stdout).lines();
    let (mut result_text, mut assistant_text, mut streamed) = (String::new(), String::new(), String::new());
    let (mut result_is_error, mut result_subtype) = (false, None::<String>);

    let read_loop = async {
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            match v["type"].as_str() {
                Some("stream_event") => {
                    let ev = &v["event"];
                    if ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                        if let Some(tok) = ev["delta"]["text"].as_str() {
                            if !tok.is_empty() {
                                streamed.push_str(tok);
                                on_token(tok.to_string());
                            }
                        }
                    }
                }
                Some("assistant") => {
                    if let Some(content) = v["message"]["content"].as_array() {
                        let mut t = String::new();
                        for b in content {
                            if b["type"] == "text" {
                                if let Some(s) = b["text"].as_str() { t.push_str(s); }
                            }
                        }
                        if !t.is_empty() { assistant_text = t; }
                    }
                }
                Some("result") => {
                    if let Some(r) = v["result"].as_str() { result_text = r.to_string(); }
                    result_is_error = v["is_error"].as_bool().unwrap_or(false);
                    result_subtype = v["subtype"].as_str().map(String::from);
                }
                _ => {}
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    // Read until EOF, a timeout, or a cancel — whichever comes first.
    let timeout = claude_timeout();
    let cancel_token = cancel.cloned().unwrap_or_default(); // a fresh token never fires
    enum End { Done(Result<()>), TimedOut, Cancelled }
    let end = tokio::select! {
        r = tokio::time::timeout(timeout, read_loop) => match r {
            Ok(inner) => End::Done(inner),
            Err(_) => End::TimedOut,
        },
        _ = cancel_token.cancelled() => End::Cancelled,
    };
    match end {
        End::Done(Ok(())) => {}
        End::Done(Err(e)) => {
            let _ = child.start_kill();
            let _ = child.wait().await; // reap
            return Err(anyhow!("error reading claude output: {e}"));
        }
        End::TimedOut => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(anyhow!(
                "claude timed out after {}s and the run was stopped (set SONGSMITH_CLAUDE_TIMEOUT_SECS to change the limit)",
                timeout.as_secs()
            ));
        }
        End::Cancelled => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            return Err(anyhow!("run cancelled"));
        }
    }

    let status = child.wait().await?;
    let stderr_buf = stderr_task.await.unwrap_or_default();
    if !status.success() {
        let err = stderr_buf.trim();
        if !err.is_empty() {
            return Err(anyhow!("claude CLI failed: {err}"));
        }
        // Empty stderr tells us nothing, so don't invent a diagnosis: the old
        // message always blamed sign-in, which sent the user checking auth
        // when the real cause was a transient failure mid-run (2026-07-29).
        // Report what we actually saw on the stream instead.
        let saw = if let Some(sub) = &result_subtype {
            format!("its last result was \"{sub}\"")
        } else if !assistant_text.trim().is_empty() || !streamed.trim().is_empty() {
            "it had started answering".to_string()
        } else {
            "it produced no output at all".to_string()
        };
        let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".into());
        return Err(anyhow!(
            "the claude CLI exited (code {code}) without an error message — {saw}. \
             Retry: this is usually transient (a service blip or a usage limit). \
             If it repeats, run `claude` once in a terminal to confirm you're signed in."
        ));
    }

    // An error-shaped `result` event (is_error / non-"success" subtype) must be
    // surfaced as the failure it is — never saved as an artifact.
    if result_is_error || result_subtype.as_deref().is_some_and(|s| s != "success") {
        let msg = if !result_text.trim().is_empty() { result_text.trim().to_string() }
            else if !assistant_text.trim().is_empty() { assistant_text.trim().to_string() }
            else { "no error detail".to_string() };
        return Err(anyhow!("claude reported an error ({}): {msg}", result_subtype.as_deref().unwrap_or("is_error")));
    }

    let out = if !result_text.trim().is_empty() { result_text }
        else if !assistant_text.trim().is_empty() { assistant_text }
        else { streamed };
    if out.trim().is_empty() {
        return Err(anyhow!("claude returned no output"));
    }
    Ok(out)
}

/// Extract the first JSON object from model output (fenced or bare).
pub fn extract_json(text: &str) -> Option<Value> {
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```") {
            if let Ok(v) = serde_json::from_str::<Value>(after[..end].trim()) {
                return Some(v);
            }
        }
    }
    let bytes = text.as_bytes();
    if let Some(open) = text.find('{') {
        let (mut depth, mut in_str, mut esc) = (0i32, false, false);
        for i in open..bytes.len() {
            let c = bytes[i] as char;
            if in_str {
                if esc { esc = false; } else if c == '\\' { esc = true; } else if c == '"' { in_str = false; }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v) = serde_json::from_str::<Value>(&text[open..=i]) { return Some(v); }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    None
}
