#[cfg(test)]
mod tests;

use crate::adapter::{EventSummary, ParsedHookEvent, ToolAdapter};

pub use crate::adapter::common::should_skip_bash_command;

pub struct ClaudeCodeAdapter;

impl ToolAdapter for ClaudeCodeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn parse_hook(&self, raw_json: &str) -> Option<ParsedHookEvent> {
        crate::adapter::common::parse_tool_hook(raw_json)
    }

    fn should_skip(&self, event: &ParsedHookEvent) -> bool {
        crate::adapter::common::should_skip_tool(&event.tool_name)
    }

    fn should_skip_bash(&self, command: &str) -> bool {
        should_skip_bash_command(command)
    }

    fn classify_event(&self, event: &ParsedHookEvent) -> Option<EventSummary> {
        crate::adapter::common::event_summary(
            &event.tool_name,
            &event.tool_input,
            &event.tool_response,
        )
    }
}
