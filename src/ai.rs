mod cli;
mod codex_cli;
mod config;
mod http;
mod pricing;
#[cfg(test)]
mod tests;
mod types;
mod usage;

use cli::call_cli;
use codex_cli::call_codex_cli;
use http::call_http;
use pricing::estimate_tokens;
use usage::record_usage;

pub use types::UsageContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiExecutor {
    Http,
    ClaudeCli,
    CodexCli,
}

/// AI call with timeout. HTTP first (fast, ~2-5s), CLI fallback (slow, ~30-60s).
pub async fn call_ai(
    system: &str,
    user_message: &str,
    ctx: UsageContext<'_>,
) -> anyhow::Result<String> {
    let result = match executor_for_operation(ctx.operation) {
        Some(AiExecutor::Http) => call_http(system, user_message).await,
        Some(AiExecutor::ClaudeCli) => call_cli(system, user_message).await,
        Some(AiExecutor::CodexCli) => call_codex_cli(system, user_message).await,
        None => call_auto(system, user_message).await,
    }?;

    let input_tokens = estimate_tokens(system) + estimate_tokens(user_message);
    let output_tokens = estimate_tokens(&result.text);
    record_usage(ctx, &result, input_tokens, output_tokens);
    Ok(result.text)
}

async fn call_auto(system: &str, user_message: &str) -> anyhow::Result<types::AiCallResult> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() || std::env::var("ANTHROPIC_AUTH_TOKEN").is_ok() {
        match call_http(system, user_message).await {
            Ok(result) => Ok(result),
            Err(http_err) => {
                crate::log::warn(
                    "ai",
                    &format!("HTTP failed, falling back to CLI: {}", http_err),
                );
                call_cli(system, user_message).await
            }
        }
    } else {
        call_cli(system, user_message).await
    }
}

fn executor_for_operation(operation: &str) -> Option<AiExecutor> {
    for key in executor_env_keys(operation) {
        if let Some(executor) = executor_from_env(key) {
            return Some(executor);
        }
    }
    None
}

fn executor_env_keys(operation: &str) -> &'static [&'static str] {
    match operation {
        "summarize" => &["REMEM_SUMMARY_EXECUTOR", "REMEM_EXECUTOR"],
        "flush" | "flush-task" => &["REMEM_FLUSH_EXECUTOR", "REMEM_EXECUTOR"],
        "compress" => &["REMEM_COMPRESS_EXECUTOR", "REMEM_EXECUTOR"],
        "dream" => &["REMEM_DREAM_EXECUTOR", "REMEM_EXECUTOR"],
        _ => &["REMEM_EXECUTOR"],
    }
}

fn executor_from_env(key: &str) -> Option<AiExecutor> {
    match std::env::var(key).ok()?.as_str() {
        "http" | "anthropic-http" | "anthropic" => Some(AiExecutor::Http),
        "cli" | "claude-cli" | "claude" => Some(AiExecutor::ClaudeCli),
        "codex-cli" | "codex" => Some(AiExecutor::CodexCli),
        _ => None,
    }
}
