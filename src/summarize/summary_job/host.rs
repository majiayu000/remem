use anyhow::Result;

pub(super) fn resolve_hook_host(host: Option<&str>) -> Result<String> {
    if let Some(host) = clean_optional(host) {
        return Ok(crate::runtime_config::normalize_host(&host));
    }
    if let Some(host) = legacy_hook_host_from_env() {
        return Ok(host);
    }
    crate::runtime_config::default_host()
}

fn legacy_hook_host_from_env() -> Option<String> {
    for key in ["REMEM_HOOK_HOST", "REMEM_CONTEXT_HOST"] {
        if let Ok(host) = std::env::var(key) {
            if let Some(host) = clean_optional(Some(&host)) {
                return Some(crate::runtime_config::normalize_host(&host));
            }
        }
    }
    for key in ["REMEM_SUMMARY_EXECUTOR", "REMEM_EXECUTOR"] {
        if let Ok(executor) = std::env::var(key) {
            match executor.trim().to_ascii_lowercase().as_str() {
                "codex-cli" | "codex" => return Some("codex-cli".to_string()),
                "claude-cli" | "claude" | "cli" => return Some("claude-code".to_string()),
                _ => {}
            }
        }
    }
    None
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
