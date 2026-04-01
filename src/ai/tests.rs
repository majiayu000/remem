use std::sync::Mutex;

use super::config::resolve_model_for_api;
use super::pricing::{estimate_cost_usd, pricing_for_model};

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
