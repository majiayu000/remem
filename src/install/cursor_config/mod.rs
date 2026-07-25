//! Cursor user-level install surface (GH-824).
//!
//! This module owns the strict whole-document parser/plan/secure-writer
//! contract for `~/.cursor/hooks.json` and `~/.cursor/mcp.json`, plus the
//! install-side `hosts.cursor` receipt. It deliberately does not reuse the
//! substring-based Claude/Codex ownership helpers in
//! `crate::install::config` (B-006/B-007 require exact structural
//! ownership).
//!
//! Contract v1 capability gates (checked against the GH-823 runtime that is
//! actually merged, not against candidate evidence):
//! - The observe bundle (`postToolUse` + `postToolUseFailure`) is absent:
//!   the merged GH-823 runtime accepts only the observed failed-`Read`
//!   failure shape and rejects other delivered failure tool names with a
//!   non-zero exit, which is not the required total
//!   capture-or-explicit-zero-write (exit 0) policy.
//! - `afterMCPExecution` is absent: GH-823 froze B-016 generic ownership.
//! - `stop` summarize is absent: `remem summarize --host cursor` stays
//!   fail-closed until GH-825's transcript reader is merged.
//! - `sessionStart` injection is blocked on Cursor 3.12.17 and `preCompact`
//!   has no approved action; neither installs.
//!
//! The managed hook component set for contract v1 is therefore empty; the
//! install surface manages exactly one MCP component and still validates,
//! preserves, and (via receipt) can later upgrade or remove hook entries.

pub(crate) mod plan;
pub(crate) mod schema;
pub(in crate::install) mod writer;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Fixed policy line shown by install output and doctor (B-005/B-018):
/// Cursor 3.12.17 evidence shows the host continues after hook failures;
/// remem-side fail-closed parsing is a different dimension.
pub(crate) const CURSOR_HOOK_FAILURE_POLICY_LINE: &str = "hook_failure_policy: host_continues";

/// Exact human-readable capability line required by B-018 in every Cursor
/// doctor state.
pub(crate) const CURSOR_SESSION_INIT_LINE: &str = "session-init: not supported on cursor";

/// User-level Cursor paths, re-exported for the doctor classifier.
pub(crate) fn cursor_hooks_path() -> PathBuf {
    crate::install::paths::cursor_hooks_path()
}

pub(crate) fn cursor_mcp_path() -> PathBuf {
    crate::install::paths::cursor_mcp_path()
}

/// Cursor is detected when the user-level config directory or either
/// user-level config file exists.
pub(crate) fn cursor_detected() -> bool {
    crate::install::paths::cursor_dir().exists()
        || cursor_hooks_path().exists()
        || cursor_mcp_path().exists()
}

/// The POSIX shell-quote renderer is only approved for macOS/Linux (B-001).
pub(crate) fn cursor_renderer_supported() -> bool {
    cfg!(unix)
}

/// GH-824 install contract version. Any schema drift in the frozen Cursor
/// hooks/MCP shapes requires a new version; the v1 parser never guesses.
pub(crate) const CURSOR_INSTALL_CONTRACT_VERSION: i64 = 1;

/// Install receipt schema version (`hosts.cursor.install_receipt`).
pub(crate) const CURSOR_RECEIPT_SCHEMA_VERSION: i64 = 1;

/// Runtime capability gates that decide which managed hook components the
/// builder may register. These reflect the merged GH-823 runtime; flipping a
/// gate requires the corresponding runtime capability to land first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CursorCapabilityGates {
    /// GH-823 total delivered-failure policy (approved failure shapes get
    /// bounded capture, every other structurally valid delivered failure
    /// tool gets an explicit exit-0 zero-write no-op).
    pub observe_total_failure_policy: bool,
    /// Stable opaque specific-event per-call ID approved for MCP-specific
    /// ownership (B-016). The merged runtime froze generic ownership.
    pub mcp_specific_per_call_id: bool,
    /// GH-825 Cursor transcript reader proven for `stop` summarize.
    pub summarize_reader_proven: bool,
}

/// Gates as satisfied by the currently merged runtime: all closed.
pub(crate) const CURRENT_CAPABILITY_GATES: CursorCapabilityGates = CursorCapabilityGates {
    observe_total_failure_policy: false,
    mcp_specific_per_call_id: false,
    summarize_reader_proven: false,
};

/// Canonical JSON digest used by receipt-bound ownership matching: SHA-256
/// over the compact `serde_json` serialization (object keys sorted).
pub(crate) fn canonical_json_digest(value: &serde_json::Value) -> String {
    let canonical = canonicalize(value);
    let serialized = serde_json::to_string(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn canonicalize(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sorted = std::collections::BTreeMap::new();
            for (key, entry) in map {
                sorted.insert(key.clone(), canonicalize(entry));
            }
            serde_json::Value::Object(sorted.into_iter().collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalize).collect())
        }
        other => other.clone(),
    }
}
