//! Strict whole-document validators for the two user-level Cursor config
//! files (GH-824 B-003/B-004). Every event and every server value is
//! validated — foreign entries are never skipped just because they are not
//! remem-owned. Errors carry a stable code plus a non-sensitive location
//! string; raw JSON values (which may contain secrets in foreign MCP `env`)
//! are never echoed.

use serde_json::{Map, Value};

/// Frozen Cursor hooks v1 event closed set (official docs, 2026-07-23).
pub(crate) const CURSOR_HOOK_EVENTS_V1: [&str; 21] = [
    "beforeShellExecution",
    "beforeMCPExecution",
    "afterShellExecution",
    "afterMCPExecution",
    "beforeReadFile",
    "afterFileEdit",
    "beforeTabFileRead",
    "afterTabFileEdit",
    "stop",
    "beforeSubmitPrompt",
    "afterAgentResponse",
    "afterAgentThought",
    "sessionStart",
    "sessionEnd",
    "preCompact",
    "subagentStart",
    "subagentStop",
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "workspaceOpen",
];

/// Events on which a hook entry may carry a `matcher` (B-003 closed list).
const MATCHER_EVENTS: [&str; 13] = [
    "preToolUse",
    "postToolUse",
    "postToolUseFailure",
    "subagentStart",
    "subagentStop",
    "beforeShellExecution",
    "afterShellExecution",
    "beforeReadFile",
    "afterFileEdit",
    "beforeSubmitPrompt",
    "stop",
    "afterAgentResponse",
    "afterAgentThought",
];

/// Events on which a hook entry may carry `loop_limit`.
const LOOP_LIMIT_EVENTS: [&str; 2] = ["stop", "subagentStop"];

/// One schema violation. `code` is stable machine-readable; `location`
/// identifies the path inside the document without echoing values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorSchemaError {
    pub code: &'static str,
    pub location: String,
}

impl std::fmt::Display for CursorSchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.code, self.location)
    }
}

fn schema_error(code: &'static str, location: impl Into<String>) -> CursorSchemaError {
    CursorSchemaError {
        code,
        location: location.into(),
    }
}

/// Validates a parsed `~/.cursor/hooks.json` document against the frozen
/// Cursor hooks v1 schema. The whole version-1 document is traversed, not
/// only remem-managed events.
pub(crate) fn validate_hooks_document(doc: &Value) -> Result<(), CursorSchemaError> {
    let Value::Object(root) = doc else {
        return Err(schema_error("hooks_root_not_object", "$"));
    };
    match root.get("version") {
        Some(Value::Number(version)) if version.as_i64() == Some(1) && !version.is_f64() => {}
        Some(_) => return Err(schema_error("hooks_version_not_integer_1", "$.version")),
        None => return Err(schema_error("hooks_version_missing", "$.version")),
    }
    let Some(hooks) = root.get("hooks") else {
        return Err(schema_error("hooks_container_missing", "$.hooks"));
    };
    let Value::Object(hooks) = hooks else {
        return Err(schema_error("hooks_container_not_object", "$.hooks"));
    };
    for (event, entries) in hooks {
        if !CURSOR_HOOK_EVENTS_V1.contains(&event.as_str()) {
            return Err(schema_error(
                "hooks_unknown_event",
                format!("$.hooks.{event}"),
            ));
        }
        let Value::Array(entries) = entries else {
            return Err(schema_error(
                "hooks_event_not_array",
                format!("$.hooks.{event}"),
            ));
        };
        for (index, entry) in entries.iter().enumerate() {
            validate_hook_entry(event, index, entry)?;
        }
    }
    Ok(())
}

fn validate_hook_entry(event: &str, index: usize, entry: &Value) -> Result<(), CursorSchemaError> {
    let location = format!("$.hooks.{event}[{index}]");
    let Value::Object(entry) = entry else {
        return Err(schema_error("hooks_entry_not_object", location));
    };

    let has_command = entry.contains_key("command");
    let has_prompt = entry.contains_key("prompt");
    match entry.get("type") {
        None => {
            // Discriminator omitted: command variant only.
            if !has_command || has_prompt {
                return Err(schema_error("hooks_entry_shape_mismatch", location));
            }
        }
        Some(Value::String(kind)) if kind == "command" => {
            if !has_command || has_prompt {
                return Err(schema_error("hooks_entry_shape_mismatch", location));
            }
        }
        Some(Value::String(kind)) if kind == "prompt" => {
            if !has_prompt || has_command {
                return Err(schema_error("hooks_entry_shape_mismatch", location));
            }
        }
        Some(_) => return Err(schema_error("hooks_entry_unknown_type", location)),
    }
    if has_command {
        let Some(Value::String(command)) = entry.get("command") else {
            return Err(schema_error("hooks_command_not_string", location));
        };
        if command.is_empty() {
            return Err(schema_error("hooks_command_empty", location));
        }
    }
    if has_prompt {
        let Some(Value::String(prompt)) = entry.get("prompt") else {
            return Err(schema_error("hooks_prompt_not_string", location));
        };
        if prompt.is_empty() {
            return Err(schema_error("hooks_prompt_empty", location));
        }
    }

    for (field, value) in entry {
        match field.as_str() {
            "type" | "command" | "prompt" => {}
            "timeout" => {
                let valid = matches!(
                    value,
                    Value::Number(number)
                        if number.as_f64().is_some_and(|v| v.is_finite() && v > 0.0)
                );
                if !valid {
                    return Err(schema_error("hooks_timeout_invalid", location));
                }
            }
            "failClosed" => {
                if !value.is_boolean() {
                    return Err(schema_error("hooks_fail_closed_not_boolean", location));
                }
            }
            "matcher" => {
                if !MATCHER_EVENTS.contains(&event) {
                    return Err(schema_error("hooks_matcher_not_allowed_on_event", location));
                }
                let valid = matches!(value, Value::String(matcher) if !matcher.is_empty());
                if !valid {
                    return Err(schema_error("hooks_matcher_invalid", location));
                }
            }
            "loop_limit" => {
                if !LOOP_LIMIT_EVENTS.contains(&event) {
                    return Err(schema_error(
                        "hooks_loop_limit_not_allowed_on_event",
                        location,
                    ));
                }
                let valid = value.is_null()
                    || matches!(
                        value,
                        Value::Number(number)
                            if number.as_i64().is_some_and(|v| v > 0) && !number.is_f64()
                    );
                if !valid {
                    return Err(schema_error("hooks_loop_limit_invalid", location));
                }
            }
            _ => return Err(schema_error("hooks_entry_unknown_field", location)),
        }
    }
    Ok(())
}

/// Validates a parsed `~/.cursor/mcp.json` document against the frozen
/// Cursor MCP v1 schema. Every server value is validated, including foreign
/// entries; the `remem` value must additionally pass ownership matching,
/// which is done by the plan layer.
pub(crate) fn validate_mcp_document(doc: &Value) -> Result<(), CursorSchemaError> {
    let Value::Object(root) = doc else {
        return Err(schema_error("mcp_root_not_object", "$"));
    };
    let Some(servers) = root.get("mcpServers") else {
        // Missing container is a valid state; the plan may create it.
        return Ok(());
    };
    let Value::Object(servers) = servers else {
        return Err(schema_error("mcp_servers_not_object", "$.mcpServers"));
    };
    for (name, server) in servers {
        validate_mcp_server(name, server)?;
    }
    Ok(())
}

fn validate_mcp_server(name: &str, server: &Value) -> Result<(), CursorSchemaError> {
    let location = format!("$.mcpServers.{name}");
    let Value::Object(server) = server else {
        return Err(schema_error("mcp_server_not_object", location));
    };
    let has_command = server.contains_key("command");
    let has_url = server.contains_key("url");
    match (has_command, has_url) {
        (true, true) => Err(schema_error("mcp_server_mixed_transport", location)),
        (false, false) => Err(schema_error("mcp_server_missing_transport", location)),
        (true, false) => validate_stdio_server(server, &location),
        (false, true) => validate_remote_server(server, &location),
    }
}

fn validate_stdio_server(
    server: &Map<String, Value>,
    location: &str,
) -> Result<(), CursorSchemaError> {
    match server.get("type") {
        None => {}
        Some(Value::String(kind)) if kind == "stdio" => {}
        Some(_) => return Err(schema_error("mcp_stdio_type_invalid", location)),
    }
    let valid_command =
        matches!(server.get("command"), Some(Value::String(cmd)) if !cmd.is_empty());
    if !valid_command {
        return Err(schema_error("mcp_stdio_command_invalid", location));
    }
    for (field, value) in server {
        match field.as_str() {
            "type" | "command" => {}
            "args" => {
                let valid = matches!(
                    value,
                    Value::Array(items) if items.iter().all(Value::is_string)
                );
                if !valid {
                    return Err(schema_error("mcp_stdio_args_invalid", location));
                }
            }
            "env" => {
                if !is_string_map(value) {
                    return Err(schema_error("mcp_stdio_env_invalid", location));
                }
            }
            "envFile" => {
                let valid = matches!(value, Value::String(path) if !path.is_empty());
                if !valid {
                    return Err(schema_error("mcp_stdio_env_file_invalid", location));
                }
            }
            _ => return Err(schema_error("mcp_stdio_unknown_field", location)),
        }
    }
    Ok(())
}

fn validate_remote_server(
    server: &Map<String, Value>,
    location: &str,
) -> Result<(), CursorSchemaError> {
    let valid_url = matches!(server.get("url"), Some(Value::String(url)) if !url.is_empty());
    if !valid_url {
        return Err(schema_error("mcp_remote_url_invalid", location));
    }
    for (field, value) in server {
        match field.as_str() {
            "url" => {}
            "headers" => {
                if !is_string_map(value) {
                    return Err(schema_error("mcp_remote_headers_invalid", location));
                }
            }
            "type" => {
                let valid = matches!(value, Value::String(kind) if kind == "http" || kind == "sse");
                if !valid {
                    return Err(schema_error("mcp_remote_type_invalid", location));
                }
            }
            "auth" => validate_remote_auth(value, location)?,
            _ => return Err(schema_error("mcp_remote_unknown_field", location)),
        }
    }
    Ok(())
}

fn validate_remote_auth(value: &Value, location: &str) -> Result<(), CursorSchemaError> {
    let Value::Object(auth) = value else {
        return Err(schema_error("mcp_remote_auth_not_object", location));
    };
    let valid_client_id =
        matches!(auth.get("CLIENT_ID"), Some(Value::String(id)) if !id.is_empty());
    if !valid_client_id {
        return Err(schema_error("mcp_remote_auth_client_id_invalid", location));
    }
    for (field, field_value) in auth {
        match field.as_str() {
            "CLIENT_ID" => {}
            "CLIENT_SECRET" => {
                let valid = matches!(field_value, Value::String(secret) if !secret.is_empty());
                if !valid {
                    return Err(schema_error(
                        "mcp_remote_auth_client_secret_invalid",
                        location,
                    ));
                }
            }
            "scopes" => {
                let valid = matches!(
                    field_value,
                    Value::Array(items)
                        if items
                            .iter()
                            .all(|item| matches!(item, Value::String(scope) if !scope.is_empty()))
                );
                if !valid {
                    return Err(schema_error("mcp_remote_auth_scopes_invalid", location));
                }
            }
            _ => return Err(schema_error("mcp_remote_auth_unknown_field", location)),
        }
    }
    Ok(())
}

fn is_string_map(value: &Value) -> bool {
    matches!(value, Value::Object(map) if map.values().all(Value::is_string))
}
