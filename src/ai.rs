use anyhow::{Context, Result};
use tokio::process::Command;

/// AI call timeout (seconds)
const AI_TIMEOUT_SECS: u64 = 90;

fn get_model_raw() -> String {
    std::env::var("REMEM_MODEL").unwrap_or_else(|_| "haiku".to_string())
}

/// Map short model names to full Anthropic API model IDs.
/// CLI handles short names itself; HTTP API needs the full ID.
fn resolve_model_for_api(short: &str) -> &str {
    match short {
        "haiku" => "claude-haiku-4-5-20251001",
        "sonnet" => "claude-sonnet-4-5-20250514",
        "opus" => "claude-opus-4-20250514",
        _ => short,
    }
}

fn get_claude_path() -> String {
    std::env::var("REMEM_CLAUDE_PATH").unwrap_or_else(|_| "claude".to_string())
}

/// AI call with timeout. HTTP first (fast, ~2-5s), CLI fallback (slow, ~30-60s).
pub async fn call_ai(system: &str, user_message: &str) -> Result<String> {
    let result = match std::env::var("REMEM_EXECUTOR").ok().as_deref() {
        Some("http") => call_http(system, user_message).await,
        Some("cli") => call_cli(system, user_message).await,
        _ => {
            // Auto: HTTP first (fast), CLI fallback
            if std::env::var("ANTHROPIC_API_KEY").is_ok()
                || std::env::var("ANTHROPIC_AUTH_TOKEN").is_ok()
            {
                match call_http(system, user_message).await {
                    Ok(text) => Ok(text),
                    Err(http_err) => {
                        crate::log::warn("ai", &format!("HTTP failed, falling back to CLI: {}", http_err));
                        call_cli(system, user_message).await
                    }
                }
            } else {
                call_cli(system, user_message).await
            }
        }
    };

    result
}

async fn call_cli(system: &str, user_message: &str) -> Result<String> {
    let model = get_model_raw();
    let claude = get_claude_path();

    let mut child = Command::new(&claude)
        .args([
            "-p",
            "--system-prompt", system,
            "--model", &model,
            "--output-format", "text",
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

    // Timeout: kill if it takes too long
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

    Ok(text)
}

async fn call_http(system: &str, user_message: &str) -> Result<String> {
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("ANTHROPIC_AUTH_TOKEN"))
        .context("ANTHROPIC_API_KEY not set")?;
    let raw = get_model_raw();
    let model = resolve_model_for_api(&raw);
    let base_url = std::env::var("ANTHROPIC_BASE_URL")
        .unwrap_or_else(|_| "https://api.anthropic.com".to_string());

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": [{"type": "text", "text": system}],
        "messages": [{"role": "user", "content": user_message}]
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(AI_TIMEOUT_SECS))
        .build()?;

    let resp = client
        .post(format!("{}/v1/messages", base_url.trim_end_matches('/')))
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Anthropic API error {}: {}", status, text);
    }

    let data: serde_json::Value = resp.json().await?;
    let text = data["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|c| c["text"].as_str())
        .unwrap_or("")
        .to_string();

    Ok(text)
}
