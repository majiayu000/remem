use crate::ai::pricing::estimate_cost_usd;
use crate::ai::types::{AiCallResult, TokenUsage, UsageContext};

pub(super) fn record_usage(
    ctx: UsageContext<'_>,
    result: &AiCallResult,
    input_tokens: i64,
    output_tokens: i64,
) {
    let operation = if ctx.operation.trim().is_empty() {
        "unknown"
    } else {
        ctx.operation
    };
    let (usage, usage_source) = match &result.usage {
        Some(usage) => (
            usage.clone(),
            result.usage_source.unwrap_or("provider_usage"),
        ),
        None => (
            TokenUsage::estimated(input_tokens, output_tokens),
            "text_estimate",
        ),
    };
    let (cost, pricing_source) = estimate_cost_usd(&result.model, &usage);
    if pricing_source == "unknown_pricing" && !usage.is_empty() {
        crate::log::warn(
            "ai",
            &format!("usage cost has unknown pricing for model {}", result.model),
        );
    }
    match crate::db::open_db().and_then(|conn| {
        crate::db::record_ai_usage(
            &conn,
            ctx.project,
            ctx.session_id,
            operation,
            result.executor,
            Some(&result.model),
            &usage,
            usage_source,
            pricing_source,
            cost,
        )?;
        Ok(())
    }) {
        Ok(_) => {}
        Err(error) => crate::log::warn("ai", &format!("usage record failed: {}", error)),
    }
}
