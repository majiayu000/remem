fn parse_env_f64(key: &str) -> Option<f64> {
    std::env::var(key).ok()?.trim().parse::<f64>().ok()
}

pub(super) fn estimate_tokens(text: &str) -> i64 {
    ((text.len() + 3) / 4) as i64
}

pub(super) fn pricing_for_model(model: &str) -> (f64, f64) {
    if let (Some(input), Some(output)) = (
        parse_env_f64("REMEM_PRICE_INPUT_PER_MTOK"),
        parse_env_f64("REMEM_PRICE_OUTPUT_PER_MTOK"),
    ) {
        return (input, output);
    }

    let model_lower = model.to_lowercase();
    let (input_default, output_default, prefix) = if model_lower.contains("opus") {
        (15.0, 75.0, "OPUS")
    } else if model_lower.contains("sonnet") {
        (3.0, 15.0, "SONNET")
    } else if model_lower.contains("haiku") {
        (0.8, 4.0, "HAIKU")
    } else {
        (0.0, 0.0, "UNKNOWN")
    };

    let input =
        parse_env_f64(&format!("REMEM_PRICE_{}_INPUT_PER_MTOK", prefix)).unwrap_or(input_default);
    let output =
        parse_env_f64(&format!("REMEM_PRICE_{}_OUTPUT_PER_MTOK", prefix)).unwrap_or(output_default);
    (input, output)
}

pub(super) fn estimate_cost_usd(model: &str, input_tokens: i64, output_tokens: i64) -> f64 {
    let (input_per_mtok, output_per_mtok) = pricing_for_model(model);
    (input_tokens as f64 / 1_000_000.0) * input_per_mtok
        + (output_tokens as f64 / 1_000_000.0) * output_per_mtok
}
