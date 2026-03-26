use std::sync::LazyLock;

/// Normalized event parsed from a hook's raw JSON input.
pub struct ParsedHookEvent {
    pub session_id: String,
    pub cwd: Option<String>,
    pub project: String,
    pub tool_name: String,
    pub tool_input: Option<serde_json::Value>,
    pub tool_response: Option<serde_json::Value>,
}

/// Structured event summary extracted from a tool call.
pub struct EventSummary {
    pub event_type: String,
    pub summary: String,
    pub detail: Option<String>,
    pub files_json: Option<String>,
    pub exit_code: Option<i32>,
}

/// Adapter trait for AI coding tool integrations.
/// Each supported tool (Claude Code, Codex, Cursor, ...) implements this.
pub trait ToolAdapter: Send + Sync {
    /// Adapter identifier (e.g. "claude-code", "codex-cli")
    fn name(&self) -> &str;

    /// Try to parse raw hook JSON into a normalized event.
    /// Returns None if this adapter cannot handle the input format.
    fn parse_hook(&self, raw_json: &str) -> Option<ParsedHookEvent>;

    /// Whether this event should be skipped entirely.
    fn should_skip(&self, event: &ParsedHookEvent) -> bool;

    /// Whether a Bash/shell command should be skipped (read-only, routine).
    fn should_skip_bash(&self, command: &str) -> bool;

    /// Extract a structured event summary from the tool call.
    fn classify_event(&self, event: &ParsedHookEvent) -> Option<EventSummary>;
}

static ADAPTERS: LazyLock<Vec<Box<dyn ToolAdapter>>> =
    LazyLock::new(|| vec![Box::new(crate::adapter_claude::ClaudeCodeAdapter)]);

/// Auto-detect adapter from raw hook JSON and parse the event.
pub fn detect_adapter(raw_json: &str) -> Option<(&'static dyn ToolAdapter, ParsedHookEvent)> {
    for adapter in ADAPTERS.iter() {
        if let Some(event) = adapter.parse_hook(raw_json) {
            return Some((adapter.as_ref(), event));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_claude_code_input() {
        let json = r#"{"session_id":"s1","cwd":"/tmp","tool_name":"Edit","tool_input":{"file_path":"x.rs"}}"#;
        let result = detect_adapter(json);
        assert!(result.is_some());
        let (adapter, event) = result.unwrap();
        assert_eq!(adapter.name(), "claude-code");
        assert_eq!(event.session_id, "s1");
        assert_eq!(event.tool_name, "Edit");
    }

    #[test]
    fn detect_unknown_input_returns_none() {
        let json = r#"{"unknown_field": true}"#;
        assert!(detect_adapter(json).is_none());
    }
}
