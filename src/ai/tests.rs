use std::sync::Mutex;

use super::config::{get_codex_model, resolve_model_for_api};
use super::pricing::{estimate_cost_usd, pricing_for_model};
use super::TokenUsage;
use super::{executor_for_operation, stable_working_dir, AiExecutor};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn with_env_vars<T>(vars: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().expect("env lock should acquire");
    let old_values = vars
        .iter()
        .map(|(key, _)| ((*key).to_string(), std::env::var(key).ok()))
        .collect::<Vec<_>>();

    for (key, value) in vars {
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    let result = f();

    for (key, value) in old_values {
        match value {
            Some(value) => unsafe { std::env::set_var(&key, value) },
            None => unsafe { std::env::remove_var(&key) },
        }
    }

    result
}

#[test]
fn resolve_model_for_api_maps_short_names() {
    assert_eq!(resolve_model_for_api("haiku"), "claude-haiku-4-5-20251001");
    assert_eq!(
        resolve_model_for_api("sonnet"),
        "claude-sonnet-4-5-20250514"
    );
    assert_eq!(resolve_model_for_api("opus"), "claude-opus-4-20250514");
    assert_eq!(resolve_model_for_api("custom-model"), "custom-model");
}

#[test]
fn pricing_for_model_uses_model_defaults() {
    with_env_vars(
        &[
            ("REMEM_PRICE_INPUT_PER_MTOK", None),
            ("REMEM_PRICE_OUTPUT_PER_MTOK", None),
            ("REMEM_PRICE_HAIKU_INPUT_PER_MTOK", None),
            ("REMEM_PRICE_HAIKU_OUTPUT_PER_MTOK", None),
        ],
        || {
            assert_eq!(pricing_for_model("haiku"), (1.0, 5.0));
        },
    );
}

#[test]
fn codex_model_defaults_to_gpt_52_and_allows_auto() {
    with_env_vars(&[("REMEM_CODEX_MODEL", None)], || {
        assert_eq!(get_codex_model().as_deref(), Some("gpt-5.2"));
    });
    with_env_vars(&[("REMEM_CODEX_MODEL", Some("auto"))], || {
        assert_eq!(get_codex_model(), None);
    });
}

#[test]
fn pricing_for_gpt_52_uses_current_flagship_rate() {
    with_env_vars(
        &[
            ("REMEM_PRICE_INPUT_PER_MTOK", None),
            ("REMEM_PRICE_OUTPUT_PER_MTOK", None),
            ("REMEM_PRICE_GPT5_CODEX_INPUT_PER_MTOK", None),
            ("REMEM_PRICE_GPT5_CODEX_OUTPUT_PER_MTOK", None),
        ],
        || {
            assert_eq!(pricing_for_model("gpt-5.2"), (1.75, 14.0));
        },
    );
}

#[test]
fn pricing_for_model_prefers_env_override() {
    with_env_vars(
        &[
            ("REMEM_PRICE_INPUT_PER_MTOK", Some("1.25")),
            ("REMEM_PRICE_OUTPUT_PER_MTOK", Some("6.5")),
        ],
        || {
            assert_eq!(pricing_for_model("sonnet"), (1.25, 6.5));
        },
    );
}

#[test]
fn estimate_cost_usd_combines_input_and_output_prices() {
    with_env_vars(
        &[
            ("REMEM_PRICE_INPUT_PER_MTOK", Some("2.0")),
            ("REMEM_PRICE_OUTPUT_PER_MTOK", Some("8.0")),
        ],
        || {
            let usage = TokenUsage::estimated(500_000, 250_000);
            let (cost, pricing_source) = estimate_cost_usd("any-model", &usage);
            assert_eq!(pricing_source, "env_override");
            assert!((cost - 3.0).abs() < f64::EPSILON);
        },
    );
}

#[test]
fn estimate_cost_usd_charges_cache_and_reasoning_separately() {
    with_env_vars(
        &[
            ("REMEM_PRICE_INPUT_PER_MTOK", None),
            ("REMEM_PRICE_OUTPUT_PER_MTOK", None),
            ("REMEM_PRICE_REASONING_PER_MTOK", None),
            ("REMEM_PRICE_CACHE_READ_PER_MTOK", None),
            ("REMEM_PRICE_CACHE_CREATION_PER_MTOK", None),
        ],
        || {
            let usage = TokenUsage {
                input_tokens: 1_000_000,
                output_tokens: 1_000_000,
                reasoning_tokens: 1_000_000,
                cache_read_tokens: 1_000_000,
                ..TokenUsage::default()
            };
            let (cost, pricing_source) = estimate_cost_usd("gpt-5.5", &usage);
            assert_eq!(pricing_source, "remem_static");
            assert!((cost - 65.5).abs() < f64::EPSILON);
        },
    );
}

#[test]
fn codex_summary_executor_falls_back_for_flush_operations() {
    with_env_vars(
        &[
            ("REMEM_EXECUTOR", None),
            ("REMEM_SUMMARY_EXECUTOR", Some("codex-cli")),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_COMPRESS_EXECUTOR", None),
            ("REMEM_DREAM_EXECUTOR", None),
        ],
        || {
            assert_eq!(
                executor_for_operation("summarize"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(
                executor_for_operation("session_rollup"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(
                executor_for_operation("observation_extract"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(
                executor_for_operation("memory_candidate"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(executor_for_operation("flush"), Some(AiExecutor::CodexCli));
            assert_eq!(
                executor_for_operation("flush-task"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(executor_for_operation("compress"), None);
        },
    );
}

#[test]
fn explicit_flush_executor_override_wins_over_codex_fallback() {
    with_env_vars(
        &[
            ("REMEM_EXECUTOR", Some("claude-cli")),
            ("REMEM_SUMMARY_EXECUTOR", Some("codex-cli")),
            ("REMEM_FLUSH_EXECUTOR", Some("http")),
            ("REMEM_COMPRESS_EXECUTOR", None),
            ("REMEM_DREAM_EXECUTOR", None),
        ],
        || {
            assert_eq!(
                executor_for_operation("summarize"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(
                executor_for_operation("session_rollup"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(executor_for_operation("flush"), Some(AiExecutor::Http));
            assert_eq!(executor_for_operation("flush-task"), Some(AiExecutor::Http));
            assert_eq!(executor_for_operation("compress"), None);
            assert_eq!(executor_for_operation("dream"), None);
            assert_eq!(executor_for_operation("other"), Some(AiExecutor::ClaudeCli));
        },
    );
}

#[test]
fn claude_summary_executor_does_not_broaden_flush_resolution() {
    with_env_vars(
        &[
            ("REMEM_EXECUTOR", None),
            ("REMEM_SUMMARY_EXECUTOR", Some("claude-cli")),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_COMPRESS_EXECUTOR", None),
            ("REMEM_DREAM_EXECUTOR", None),
        ],
        || {
            assert_eq!(
                executor_for_operation("summarize"),
                Some(AiExecutor::ClaudeCli)
            );
            assert_eq!(
                executor_for_operation("session_rollup"),
                Some(AiExecutor::ClaudeCli)
            );
            assert_eq!(executor_for_operation("flush"), None);
            assert_eq!(executor_for_operation("flush-task"), None);
        },
    );
}

#[test]
fn legacy_global_executor_applies_to_extraction_operations() {
    with_env_vars(
        &[
            ("REMEM_EXECUTOR", Some("codex-cli")),
            ("REMEM_SUMMARY_EXECUTOR", None),
            ("REMEM_FLUSH_EXECUTOR", None),
            ("REMEM_COMPRESS_EXECUTOR", None),
            ("REMEM_DREAM_EXECUTOR", None),
        ],
        || {
            assert_eq!(
                executor_for_operation("summarize"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(
                executor_for_operation("session_rollup"),
                Some(AiExecutor::CodexCli)
            );
            assert_eq!(executor_for_operation("flush"), None);
            assert_eq!(executor_for_operation("flush-task"), None);
            assert_eq!(executor_for_operation("compress"), None);
            assert_eq!(executor_for_operation("dream"), None);
        },
    );
}

#[test]
fn stable_working_dir_uses_data_dir_even_if_caller_cwd_disappears() {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("ai-stable-cwd");

    let got = stable_working_dir();

    assert_eq!(got, data_dir.path);
    assert!(got.is_dir());
}
