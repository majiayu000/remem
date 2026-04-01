use anyhow::{Context, Result};
use tokio::process::Command;

use crate::ai::config::{get_claude_path, get_model_raw};
use crate::ai::types::{AiCallResult, AI_TIMEOUT_SECS};

pub(super) async fn call_cli(system: &str, user_message: &str) -> Result<AiCallResult> {
    let model = get_model_raw();
    let claude = get_claude_path();

    let mut child = Command::new(&claude)
        .args([
            "-p",
            "--system-prompt",
            system,
            "--model",
            &model,
            "--output-format",
            "text",
            "--no-session-persistence",
        ])
        .env_remove("CLAUDECODE")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to spawn '{}' — is Claude Code installed?", claude))?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(user_message.as_bytes()).await?;
    }

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(AI_TIMEOUT_SECS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("claude CLI timed out after {}s", AI_TIMEOUT_SECS))??;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("claude CLI exited {}: {}", output.status, stderr);
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        anyhow::bail!("claude CLI returned empty response");
    }

    Ok(AiCallResult {
        text,
        executor: "cli",
        model,
    })
}
