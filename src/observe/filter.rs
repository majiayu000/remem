pub(super) fn event_skip_reason(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
    has_commit_evidence: bool,
) -> Option<&'static str> {
    event_skip_reason_with_codex_bash_enabled(
        adapter,
        event,
        has_commit_evidence,
        codex_bash_observe_enabled(),
    )
}

fn event_skip_reason_with_codex_bash_enabled(
    adapter: &dyn crate::adapter::ToolAdapter,
    event: &crate::adapter::ParsedHookEvent,
    has_commit_evidence: bool,
    codex_bash_enabled: bool,
) -> Option<&'static str> {
    if adapter.should_skip(event) {
        return Some("adapter_skip");
    }
    if event.tool_name != "Bash" {
        return None;
    }

    let command = event
        .tool_input
        .as_ref()
        .and_then(|value| value.get("command"))
        .and_then(serde_json::Value::as_str);
    let supported_commit = command.is_some_and(crate::git_evidence::is_supported_commit_command);

    if adapter.name() == "codex-cli"
        && !codex_bash_enabled
        && !has_commit_evidence
        && !supported_commit
    {
        return Some("codex_bash_disabled");
    }

    if !has_commit_evidence
        && !supported_commit
        && command.is_some_and(|command| adapter.should_skip_bash(command))
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

#[cfg(test)]
mod tests {
    use crate::adapter::{codex::CodexAdapter, ParsedHookEvent};

    use super::event_skip_reason_with_codex_bash_enabled;

    fn codex_bash_event(command: &str) -> ParsedHookEvent {
        ParsedHookEvent {
            session_id: "session".to_string(),
            cwd: Some("/tmp".to_string()),
            project: "/tmp".to_string(),
            reference_time_epoch: None,
            tool_name: "Bash".to_string(),
            tool_input: Some(serde_json::json!({"command": command})),
            tool_response: Some(serde_json::json!({"exitCode": 0})),
        }
    }

    #[test]
    fn quiet_commit_bypasses_disabled_codex_bash_capture_filter() {
        assert_eq!(
            event_skip_reason_with_codex_bash_enabled(
                &CodexAdapter,
                &codex_bash_event("git commit -q -m done"),
                false,
                false,
            ),
            None
        );
    }

    #[test]
    fn proven_commit_bypasses_read_only_prefix_filter() {
        assert_eq!(
            event_skip_reason_with_codex_bash_enabled(
                &CodexAdapter,
                &codex_bash_event("git status --short && git commit -m done"),
                true,
                false,
            ),
            None
        );
    }
}
