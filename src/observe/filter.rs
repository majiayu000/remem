pub(super) fn event_skip_reason(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
    has_commit_evidence: bool,
) -> Option<&'static str> {
    if adapter.should_skip(event) {
        return Some("adapter_skip");
    }
    if event.tool_name != "Bash" {
        return None;
    }

    if adapter.name() == "codex-cli" && !codex_bash_observe_enabled() && !has_commit_evidence {
        return Some("codex_bash_disabled");
    }

    if event
        .tool_input
        .as_ref()
        .and_then(|value| value["command"].as_str())
        .is_some_and(|command| adapter.should_skip_bash(command))
    {
        return Some("bash_read_only");
    }

    None
}

pub(super) fn skip_detail(event: &crate::adapter::ParsedHookEvent) -> Option<String> {
    event
        .tool_input
        .as_ref()
        .and_then(|value| value["command"].as_str())
        .map(|command| crate::adapter::common::redact_hook_payload_preview(command, 240))
}

fn codex_bash_observe_enabled() -> bool {
    std::env::var("REMEM_ENABLE_CODEX_BASH_OBSERVE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
