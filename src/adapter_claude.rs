mod bash;
mod classify;
mod constants;
mod hook;
#[cfg(test)]
mod tests;

use crate::adapter::{EventSummary, ParsedHookEvent, ToolAdapter};

pub use bash::should_skip_bash_command;

pub struct ClaudeCodeAdapter;

impl ToolAdapter for ClaudeCodeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn parse_hook(&self, raw_json: &str) -> Option<ParsedHookEvent> {
        hook::parse_hook(raw_json)
    }

    fn should_skip(&self, event: &ParsedHookEvent) -> bool {
        let name = event.tool_name.as_str();
        constants::SKIP_TOOLS.contains(&name) || !constants::ACTION_TOOLS.contains(&name)
    }

    fn should_skip_bash(&self, command: &str) -> bool {
        should_skip_bash_command(command)
    }

    fn classify_event(&self, event: &ParsedHookEvent) -> Option<EventSummary> {
        classify::event_summary(&event.tool_name, &event.tool_input, &event.tool_response)
    }
}
