pub(super) fn get_model_raw() -> String {
    std::env::var("REMEM_MODEL").unwrap_or_else(|_| "haiku".to_string())
}

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

pub(super) fn get_claude_path() -> String {
    std::env::var("REMEM_CLAUDE_PATH").unwrap_or_else(|_| "claude".to_string())
}

pub(super) fn get_codex_path() -> String {
    std::env::var("REMEM_CODEX_PATH").unwrap_or_else(|_| "codex".to_string())
}

pub(super) fn get_codex_model() -> Option<String> {
    std::env::var("REMEM_CODEX_MODEL")
        .ok()
        .filter(|model| !model.trim().is_empty())
}
