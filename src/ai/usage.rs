use crate::ai::pricing::estimate_cost_usd;
use crate::ai::types::{AiCallResult, UsageContext};

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
    let cost = estimate_cost_usd(&result.model, input_tokens, output_tokens);
    match crate::db::open_db().and_then(|conn| {
        crate::db::record_ai_usage(
            &conn,
            ctx.project,
            operation,
            result.executor,
            Some(&result.model),
            input_tokens,
            output_tokens,
            cost,
        )?;
        Ok(())
    }) {
        Ok(_) => {}
        Err(error) => crate::log::warn("ai", &format!("usage record failed: {}", error)),
    }
}
