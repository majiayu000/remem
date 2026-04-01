mod cli;
mod config;
mod http;
mod pricing;
#[cfg(test)]
mod tests;
mod types;
mod usage;

use cli::call_cli;
use http::call_http;
use pricing::estimate_tokens;
use usage::record_usage;

pub use types::UsageContext;

/// AI call with timeout. HTTP first (fast, ~2-5s), CLI fallback (slow, ~30-60s).
pub async fn call_ai(
    system: &str,
    user_message: &str,
    ctx: UsageContext<'_>,
) -> anyhow::Result<String> {
    let result = match std::env::var("REMEM_EXECUTOR").ok().as_deref() {
        Some("http") => call_http(system, user_message).await,
        Some("cli") => call_cli(system, user_message).await,
        _ => {
            if std::env::var("ANTHROPIC_API_KEY").is_ok()
                || std::env::var("ANTHROPIC_AUTH_TOKEN").is_ok()
            {
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
    }?;

    let input_tokens = estimate_tokens(system) + estimate_tokens(user_message);
    let output_tokens = estimate_tokens(&result.text);
    record_usage(ctx, &result, input_tokens, output_tokens);
    Ok(result.text)
}
