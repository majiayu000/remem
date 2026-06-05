/// Map short model names to full Anthropic API model IDs.
/// CLI handles short names itself; HTTP API needs the full ID.
pub(super) fn resolve_model_for_api(short: &str) -> &str {
    match short {
        "haiku" => "claude-haiku-4-5-20251001",
        "sonnet" => "claude-sonnet-4-5-20250514",
        "opus" => "claude-opus-4-20250514",
        _ => short,
    }
}
