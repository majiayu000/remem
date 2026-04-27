use std::sync::Mutex;

use super::config::resolve_model_for_api;
use super::pricing::{estimate_cost_usd, pricing_for_model};
use super::{executor_for_operation, AiExecutor};

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
            assert_eq!(pricing_for_model("haiku"), (0.8, 4.0));
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
            let cost = estimate_cost_usd("any-model", 500_000, 250_000);
            assert!((cost - 3.0).abs() < f64::EPSILON);
        },
    );
}

#[test]
fn summary_executor_override_does_not_leak_to_flush() {
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
            assert_eq!(executor_for_operation("flush"), None);
            assert_eq!(executor_for_operation("flush-task"), None);
            assert_eq!(executor_for_operation("compress"), None);
        },
    );
}

#[test]
fn operation_executor_overrides_do_not_use_global_fallback_for_background_jobs() {
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
            assert_eq!(executor_for_operation("flush"), Some(AiExecutor::Http));
            assert_eq!(executor_for_operation("compress"), None);
            assert_eq!(executor_for_operation("dream"), None);
            assert_eq!(executor_for_operation("other"), Some(AiExecutor::ClaudeCli));
        },
    );
}

#[test]
fn legacy_global_executor_only_affects_summary() {
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
            assert_eq!(executor_for_operation("flush"), None);
            assert_eq!(executor_for_operation("flush-task"), None);
            assert_eq!(executor_for_operation("compress"), None);
            assert_eq!(executor_for_operation("dream"), None);
        },
    );
}
