use serde::Deserialize;

use crate::adapter::ParsedHookEvent;
use crate::db;

#[derive(Debug, Deserialize)]
struct HookInput {
    session_id: Option<String>,
    cwd: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    tool_response: Option<serde_json::Value>,
}

pub(super) fn parse_hook(raw_json: &str) -> Option<ParsedHookEvent> {
    let hook: HookInput = serde_json::from_str(raw_json).ok()?;
    let session_id = hook.session_id?;
    let cwd = hook.cwd;
    let project = db::project_from_cwd(cwd.as_deref().unwrap_or("."));
    Some(ParsedHookEvent {
        session_id,
        cwd,
        project,
        tool_name: hook.tool_name.unwrap_or_else(|| "unknown".into()),
        tool_input: hook.tool_input,
        tool_response: hook.tool_response,
    })
}
