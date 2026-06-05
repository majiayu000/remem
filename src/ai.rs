mod cli;
mod codex_cli;
mod codex_usage;
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

pub(crate) use types::TokenUsage;
pub use types::UsageContext;

/// AI call with timeout. Executor/model/path are resolved from remem config.
pub async fn call_ai(
    system: &str,
    user_message: &str,
    ctx: UsageContext<'_>,
) -> anyhow::Result<String> {
    let profile = crate::runtime_config::resolve_memory_ai_profile(
        crate::runtime_config::MemoryAiSelection {
            host: ctx.host,
            profile: ctx.profile,
        },
    )?;
    let result = match profile.executor {
        crate::runtime_config::MemoryAiExecutor::Http => {
            call_http(system, user_message, &profile).await
        }
        crate::runtime_config::MemoryAiExecutor::ClaudeCli => {
            call_cli(system, user_message, &profile).await
        }
        crate::runtime_config::MemoryAiExecutor::CodexCli => {
            call_codex_cli(system, user_message, &profile).await
        }
    }?;

    let input_tokens = estimate_tokens(system) + estimate_tokens(user_message);
    let output_tokens = estimate_tokens(&result.text);
    record_usage(ctx, &result, input_tokens, output_tokens);
    Ok(result.text)
}

fn stable_working_dir() -> std::path::PathBuf {
    let data_dir = crate::db::data_dir();
    match std::fs::create_dir_all(&data_dir) {
        Ok(()) => data_dir,
        Err(err) => {
            crate::log::warn(
                "ai",
                &format!(
                    "failed to create AI working dir {}: {}; falling back to temp dir",
                    data_dir.display(),
                    err
                ),
            );
            std::env::temp_dir()
        }
    }
}
