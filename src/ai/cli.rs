use anyhow::{Context, Result};
use tokio::process::Command;

use crate::ai::types::{AiCallResult, AI_TIMEOUT_SECS};
use crate::runtime_config::ResolvedMemoryAiProfile;

pub(super) async fn call_cli(
    system: &str,
    user_message: &str,
    profile: &ResolvedMemoryAiProfile,
) -> Result<AiCallResult> {
    let model = profile.model.as_deref().unwrap_or("haiku");
    let claude = profile.cli_path.as_deref().unwrap_or("claude");
    let working_dir = super::stable_working_dir();

    let mut child = Command::new(claude)
        .args([
            "-p",
            "--system-prompt",
            system,
            "--model",
            model,
            "--output-format",
            "text",
            "--no-session-persistence",
        ])
        .current_dir(&working_dir)
        .env("REMEM_DISABLE_HOOKS", "1")
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
        model: model.to_string(),
        usage: None,
        usage_source: None,
    })
}
