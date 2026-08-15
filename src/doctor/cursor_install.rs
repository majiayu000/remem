//! Cursor host doctor state model (GH-824 B-018/B-019).
//!
//! Reports the install-surface dimensions separately instead of collapsing
//! them into one "installed" boolean: `detected`, `configured`,
//! `configured_mode`, `malformed`, `partial_state`, `drift`, `collision`,
//! per-capability `effective`, `hook_failure_policy`, and the fixed
//! `session-init: not supported on cursor` capability line.

use serde_json::Value;
use std::path::Path;

use super::types::{Check, Status};
use crate::install::cursor_config::plan::{
    classify_remem_mcp, managed_cursor_mcp_entry, parse_receipt_from_config, CursorInstallReceipt,
    McpOwnership,
};
use crate::install::cursor_config::schema::{validate_hooks_document, validate_mcp_document};
use crate::install::cursor_config::{
    cursor_detected, cursor_hooks_path, cursor_mcp_path, CURSOR_HOOK_FAILURE_POLICY_LINE,
    CURSOR_SESSION_INIT_LINE,
};

/// Per-capability `effective` baseline from the adopted GH-822 / PR #914
/// Cursor 3.12.17 evidence (B-019). The observe capture entry is never a
/// context producer, so `postToolUse_managed_context` stays
/// `not_configured` until an independent packet installs one.
const EFFECTIVE_CAPABILITY_LINES: &str =
    "effective: unknown (evidence: GH-822 PR #914, Cursor 3.12.17); \
     postToolUse_delivery: proven; postToolUse_managed_context: not_configured; \
     sessionStart: blocked; stop: unknown; preCompact: unknown";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfiguredMode {
    Full,
    HooksOnly,
    None,
    Unknown,
}

impl ConfiguredMode {
    fn label(self) -> &'static str {
        match self {
            ConfiguredMode::Full => "full",
            ConfiguredMode::HooksOnly => "hooks_only",
            ConfiguredMode::None => "none",
            ConfiguredMode::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
struct CursorDoctorState {
    detected: bool,
    configured: bool,
    configured_mode: ConfiguredMode,
    malformed: Option<String>,
    partial_state: Option<&'static str>,
    drift: bool,
    collision: bool,
}

pub(super) fn check_cursor_install() -> Check {
    let state = classify(&cursor_hooks_path(), &cursor_mcp_path());
    render(state)
}

fn classify(hooks_path: &Path, mcp_path: &Path) -> CursorDoctorState {
    let mut state = CursorDoctorState {
        detected: cursor_detected(),
        configured: false,
        configured_mode: ConfiguredMode::None,
        malformed: None,
        partial_state: None,
        drift: false,
        collision: false,
    };
    if !state.detected {
        return state;
    }

    let hooks_doc = match read_json(hooks_path) {
        Ok(doc) => doc,
        Err(detail) => {
            state.malformed = Some(detail);
            state.configured_mode = ConfiguredMode::Unknown;
            return state;
        }
    };
    if let Some(doc) = hooks_doc.as_ref() {
        if let Err(error) = validate_hooks_document(doc) {
            state.malformed = Some(format!("{}: {error}", hooks_path.display()));
            state.configured_mode = ConfiguredMode::Unknown;
            return state;
        }
    }
    let mcp_doc = match read_json(mcp_path) {
        Ok(doc) => doc,
        Err(detail) => {
            state.malformed = Some(detail);
            state.configured_mode = ConfiguredMode::Unknown;
            return state;
        }
    };
    if let Some(doc) = mcp_doc.as_ref() {
        if let Err(error) = validate_mcp_document(doc) {
            state.malformed = Some(format!("{}: {error}", mcp_path.display()));
            state.configured_mode = ConfiguredMode::Unknown;
            return state;
        }
    }

    let receipt = match load_receipt() {
        Ok(receipt) => receipt,
        Err(detail) => {
            state.malformed = Some(detail);
            state.configured_mode = ConfiguredMode::Unknown;
            return state;
        }
    };

    let remem_entry = mcp_doc
        .as_ref()
        .and_then(|doc| doc.get("mcpServers"))
        .and_then(|servers| servers.get("remem"));
    let mcp_present = remem_entry.is_some();

    match &receipt {
        None => {
            if mcp_present {
                // A remem key exists but no receipt proves intent. An exact
                // current-builder match cannot be classified as configured
                // without knowing the binary path this machine expects, so
                // report ambiguous partial state; anything else is a
                // collision.
                state.configured_mode = ConfiguredMode::Unknown;
                state.partial_state =
                    Some("mcpServers.remem exists but no install receipt records it");
            } else {
                state.configured_mode = ConfiguredMode::None;
            }
        }
        Some(receipt) => {
            let ownership = classify_remem_mcp(remem_entry, &receipt.binary_path, Some(receipt));
            let current_expected = remem_entry
                .is_some_and(|entry| *entry == managed_cursor_mcp_entry(&receipt.binary_path));
            match (receipt.mode.as_str(), mcp_present) {
                ("full", true) => match ownership {
                    McpOwnership::OwnedCurrent | McpOwnership::OwnedReceipt if current_expected => {
                        state.configured = true;
                        state.configured_mode = ConfiguredMode::Full;
                    }
                    McpOwnership::OwnedCurrent | McpOwnership::OwnedReceipt => {
                        state.drift = true;
                        state.configured_mode = ConfiguredMode::Unknown;
                    }
                    _ => {
                        state.collision = true;
                        state.configured_mode = ConfiguredMode::Unknown;
                    }
                },
                ("full", false) => {
                    state.configured_mode = ConfiguredMode::Unknown;
                    state.partial_state =
                        Some("receipt records mode=full but mcpServers.remem is missing");
                }
                ("hooks_only", false) => {
                    // Intentional hooks-only: receipt matches file state and
                    // is explicitly not partial (B-018).
                    state.configured = true;
                    state.configured_mode = ConfiguredMode::HooksOnly;
                }
                ("hooks_only", true) => match ownership {
                    McpOwnership::Collision => {
                        state.collision = true;
                        state.configured_mode = ConfiguredMode::Unknown;
                    }
                    _ => {
                        state.configured_mode = ConfiguredMode::Unknown;
                        state.partial_state =
                            Some("receipt records mode=hooks_only but a remem MCP entry exists");
                    }
                },
                _ => {
                    state.configured_mode = ConfiguredMode::Unknown;
                    state.partial_state = Some("receipt mode is not recognized");
                }
            }
        }
    }
    state
}

fn render(state: CursorDoctorState) -> Check {
    let status = if state.malformed.is_some() || state.collision || state.partial_state.is_some() {
        Status::Fail
    } else if state.drift || (state.detected && !state.configured) {
        Status::Warn
    } else {
        Status::Ok
    };

    let mut detail = format!(
        "detected={} configured={} configured_mode={}",
        state.detected,
        state.configured,
        state.configured_mode.label()
    );
    if let Some(malformed) = &state.malformed {
        detail.push_str(&format!(
            "\nmalformed: {malformed} (files were left unchanged)"
        ));
    }
    if let Some(partial) = state.partial_state {
        detail.push_str(&format!(
            "\npartial_state: {partial}; rerun `remem install --target cursor` or `remem uninstall --target cursor` to converge"
        ));
    }
    if state.collision {
        detail.push_str(
            "\ncollision: an entry remem cannot prove it owns occupies a managed key; remem will not modify or delete it",
        );
    }
    if state.drift {
        detail.push_str(
            "\ndrift: managed entry differs from the current builder shape; rerun `remem install --target cursor`",
        );
    }
    detail.push_str(&format!("\n{CURSOR_HOOK_FAILURE_POLICY_LINE}"));
    detail.push_str(&format!("\n{EFFECTIVE_CAPABILITY_LINES}"));
    detail.push_str(&format!(
        "\nsession_init: unsupported\n{CURSOR_SESSION_INIT_LINE}"
    ));

    Check::new("Cursor install", status, detail)
}

fn read_json(path: &Path) -> Result<Option<Value>, String> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    match serde_json::from_str(&content) {
        Ok(doc) => Ok(Some(doc)),
        Err(error) => Err(format!("cannot parse {}: {error}", path.display())),
    }
}

fn load_receipt() -> Result<Option<CursorInstallReceipt>, String> {
    let path = crate::runtime_config::config_path().map_err(|error| error.to_string())?;
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    parse_receipt_from_config(&doc).map_err(|error| format!("{}: {error:#}", path.display()))
}

#[cfg(test)]
#[path = "cursor_install/tests.rs"]
mod tests;
