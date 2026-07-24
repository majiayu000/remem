//! Side-effect-free Cursor install/uninstall planning (GH-824 B-009).
//!
//! `CursorConfigPlan::build` reads and fully validates both Cursor files and
//! the canonical runtime config/receipt before any write is planned. All
//! operations and modes (`install`, `uninstall`, `--hooks-only`, `--dry-run`,
//! doctor) share this parser and its error codes.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use toml_edit::{value, DocumentMut, Item};

use super::schema::{validate_hooks_document, validate_mcp_document};
use super::{
    canonical_json_digest, CursorCapabilityGates, CURRENT_CAPABILITY_GATES,
    CURSOR_INSTALL_CONTRACT_VERSION, CURSOR_RECEIPT_SCHEMA_VERSION,
};
use crate::install::paths::{cursor_hooks_path, cursor_mcp_path};

/// Operation being planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorOperation {
    Install { hooks_only: bool },
    Uninstall,
}

/// Per-target plan outcome (B-016 dry-run vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileAction {
    Add,
    Remove,
    Replace,
    NoOp,
}

impl FileAction {
    pub(crate) fn label(self) -> &'static str {
        match self {
            FileAction::Add => "add",
            FileAction::Remove => "remove",
            FileAction::Replace => "replace",
            FileAction::NoOp => "no-op",
        }
    }
}

/// Raw-bytes snapshot used for concurrent-edit final comparison (B-012).
/// `bytes == None` means the file did not exist at plan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileSnapshot {
    pub path: PathBuf,
    pub bytes: Option<Vec<u8>>,
}

impl FileSnapshot {
    pub(crate) fn capture(path: PathBuf) -> Result<Self> {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("read {}", path.display()));
            }
        };
        Ok(Self { path, bytes })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FilePlan {
    pub snapshot: FileSnapshot,
    pub action: FileAction,
    /// Non-sensitive reason rendered in dry-run output.
    pub reason: &'static str,
    /// Planned full-file bytes; `Some` exactly when `action` mutates.
    pub new_bytes: Option<Vec<u8>>,
}

/// Versioned, non-sensitive install receipt (B-006/B-015), stored as
/// canonical JSON in the `memory_ai.hosts.cursor.install_receipt` string key
/// of the runtime config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CursorInstallReceipt {
    pub schema_version: i64,
    pub contract_version: i64,
    /// `full` or `hooks_only`.
    pub mode: String,
    /// Canonical unquoted binary path of the last successful install.
    pub binary_path: String,
    pub components: Vec<ReceiptComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReceiptComponent {
    /// Stable component id, e.g. `mcp_server_v1`.
    pub component: String,
    /// `mcp` or `hook`.
    pub kind: String,
    /// MCP key (`mcpServers.remem`) or hooks event name.
    pub key: String,
    /// Fixed command tail for hook components (e.g. ` observe --host cursor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_tail: Option<String>,
    /// Rendered command for hook components.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i64>,
    /// Canonical JSON SHA-256 digest of the whole managed entry.
    pub digest: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CursorConfigPlan {
    pub operation: CursorOperation,
    pub hooks: FilePlan,
    pub mcp: FilePlan,
    pub runtime_config: FilePlan,
}

/// Exact Cursor MCP builder for contract v1 (B-004). Cursor-specific: never
/// reuse the Claude builder, which may grow host-specific fields.
pub(crate) fn managed_cursor_mcp_entry(bin: &str) -> Value {
    json!({
        "type": "stdio",
        "command": bin,
        "args": ["mcp"],
    })
}

/// Managed hook builder for contract v1 (B-005). Every component is behind a
/// runtime capability gate; with the currently merged GH-823 runtime all
/// gates are closed, so the component list is empty. The shapes are kept
/// here (not deleted) because they are the frozen contract for the gates.
pub(crate) fn managed_hook_components(
    bin: &str,
    gates: CursorCapabilityGates,
) -> Vec<ReceiptComponent> {
    let mut components = Vec::new();
    let quoted = crate::install::config::shell_quote(bin);
    let mut push = |component: &str, event: &str, tail: &str| {
        let rendered = format!("{quoted}{tail}");
        let entry = json!({ "command": rendered, "timeout": 120 });
        components.push(ReceiptComponent {
            component: component.to_string(),
            kind: "hook".to_string(),
            key: event.to_string(),
            command_tail: Some(tail.to_string()),
            rendered_command: Some(rendered),
            timeout: Some(120),
            digest: canonical_json_digest(&entry),
        });
    };
    // The observe bundle is atomic: success capture never installs without
    // the total delivered-failure policy (B-005).
    if gates.observe_total_failure_policy {
        push(
            "observe_generic_success_v1",
            "postToolUse",
            " observe --host cursor",
        );
        push(
            "observe_generic_failure_v1",
            "postToolUseFailure",
            " observe --host cursor",
        );
        if gates.mcp_specific_per_call_id {
            push(
                "observe_mcp_specific_v1",
                "afterMCPExecution",
                " observe --host cursor",
            );
        }
    }
    if gates.summarize_reader_proven {
        push("summarize_stop_v1", "stop", " summarize --host cursor");
    }
    components
}

/// Builds the receipt that a successful apply commits (B-015).
pub(crate) fn build_receipt(bin: &str, hooks_only: bool) -> CursorInstallReceipt {
    let mut components = managed_hook_components(bin, CURRENT_CAPABILITY_GATES);
    if !hooks_only {
        let entry = managed_cursor_mcp_entry(bin);
        components.insert(
            0,
            ReceiptComponent {
                component: "mcp_server_v1".to_string(),
                kind: "mcp".to_string(),
                key: "mcpServers.remem".to_string(),
                command_tail: None,
                rendered_command: None,
                timeout: None,
                digest: canonical_json_digest(&entry),
            },
        );
    }
    CursorInstallReceipt {
        schema_version: CURSOR_RECEIPT_SCHEMA_VERSION,
        contract_version: CURSOR_INSTALL_CONTRACT_VERSION,
        mode: if hooks_only { "hooks_only" } else { "full" }.to_string(),
        binary_path: bin.to_string(),
        components,
    }
}

/// Ownership classification for `mcpServers.remem` (B-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpOwnership {
    Absent,
    OwnedCurrent,
    /// Exact receipt-bound old-binary-path match; eligible for upgrade.
    OwnedReceipt,
    Collision,
}

pub(crate) fn classify_remem_mcp(
    entry: Option<&Value>,
    bin: &str,
    receipt: Option<&CursorInstallReceipt>,
) -> McpOwnership {
    let Some(entry) = entry else {
        return McpOwnership::Absent;
    };
    if *entry == managed_cursor_mcp_entry(bin) {
        return McpOwnership::OwnedCurrent;
    }
    if let Some(receipt) = receipt {
        let recorded = receipt
            .components
            .iter()
            .find(|component| component.kind == "mcp" && component.key == "mcpServers.remem");
        if let Some(component) = recorded {
            let expected_old = managed_cursor_mcp_entry(&receipt.binary_path);
            if *entry == expected_old && canonical_json_digest(entry) == component.digest {
                return McpOwnership::OwnedReceipt;
            }
        }
    }
    McpOwnership::Collision
}

pub(crate) fn parse_receipt_from_config(doc: &DocumentMut) -> Result<Option<CursorInstallReceipt>> {
    let Some(raw) = doc
        .get("memory_ai")
        .and_then(Item::as_table)
        .and_then(|table| table.get("hosts"))
        .and_then(Item::as_table)
        .and_then(|hosts| hosts.get(crate::runtime_config::CURSOR_HOST))
        .and_then(Item::as_table)
        .and_then(|table| table.get("install_receipt"))
    else {
        return Ok(None);
    };
    let Some(raw) = raw.as_str() else {
        bail!("cursor install receipt is not a string (code=receipt_invalid)");
    };
    let receipt: CursorInstallReceipt = serde_json::from_str(raw)
        .context("cursor install receipt is not valid receipt JSON (code=receipt_invalid)")?;
    if receipt.schema_version != CURSOR_RECEIPT_SCHEMA_VERSION
        || receipt.contract_version != CURSOR_INSTALL_CONTRACT_VERSION
    {
        bail!(
            "cursor install receipt has unsupported schema/contract version (code=receipt_unsupported_version)"
        );
    }
    if !matches!(receipt.mode.as_str(), "full" | "hooks_only") {
        bail!("cursor install receipt has unknown mode (code=receipt_invalid)");
    }
    Ok(Some(receipt))
}

fn parse_json_snapshot(snapshot: &FileSnapshot) -> Result<Option<Value>> {
    let Some(bytes) = snapshot.bytes.as_ref() else {
        return Ok(None);
    };
    let text = std::str::from_utf8(bytes).with_context(|| {
        format!(
            "{} is not valid UTF-8 (code=malformed_utf8)",
            snapshot.path.display()
        )
    })?;
    let doc: Value = serde_json::from_str(text).with_context(|| {
        format!(
            "{} is not valid JSON (code=malformed_json)",
            snapshot.path.display()
        )
    })?;
    Ok(Some(doc))
}

fn render_json_bytes(doc: &Value) -> Result<Vec<u8>> {
    let mut text = serde_json::to_string_pretty(doc)?;
    text.push('\n');
    Ok(text.into_bytes())
}

/// Strict install-side validation + defaults materialization for the
/// `[memory_ai.hosts.cursor]` section (B-002). Only missing fields are
/// filled; explicit values must be type- and closed-set-valid, and
/// `capture_adapter` accepts exactly `cursor`.
fn plan_cursor_host_section(doc: &mut DocumentMut) -> Result<()> {
    let memory_ai = doc
        .entry("memory_ai")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("memory_ai exists but is not a table (code=runtime_config_malformed)")?;
    let hosts = memory_ai
        .entry("hosts")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("memory_ai.hosts exists but is not a table (code=runtime_config_malformed)")?;
    let table = hosts
        .entry(crate::runtime_config::CURSOR_HOST)
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context(
            "memory_ai.hosts.cursor exists but is not a table (code=runtime_config_malformed)",
        )?;

    match table.get("memory_profile") {
        None => table["memory_profile"] = value("codex"),
        Some(item) => {
            let valid = item
                .as_str()
                .is_some_and(|profile| !profile.trim().is_empty());
            if !valid {
                bail!("hosts.cursor.memory_profile must be a non-empty string (code=host_config_invalid)");
            }
        }
    }
    match table.get("context_gate") {
        None => table["context_gate"] = value("strict"),
        Some(item) => {
            let valid = item
                .as_str()
                .is_some_and(|gate| matches!(gate, "strict" | "auto" | "off"));
            if !valid {
                bail!("hosts.cursor.context_gate must be one of strict|auto|off (code=host_config_invalid)");
            }
        }
    }
    match table.get("context_color") {
        None => table["context_color"] = value(true),
        Some(item) => {
            if item.as_bool().is_none() {
                bail!("hosts.cursor.context_color must be a boolean (code=host_config_invalid)");
            }
        }
    }
    match table.get("capture_adapter") {
        None => table["capture_adapter"] = value(crate::cursor_hook::CURSOR_HOST),
        Some(item) => {
            // Host identity boundary: only the exact string `cursor` is
            // acceptable, even when the type is correct (B-002).
            if item.as_str() != Some(crate::cursor_hook::CURSOR_HOST) {
                bail!(
                    "hosts.cursor.capture_adapter accepts only the exact string \"cursor\" (code=capture_adapter_invalid)"
                );
            }
        }
    }
    Ok(())
}

impl CursorConfigPlan {
    /// Reads and validates both Cursor files plus the runtime config, then
    /// produces the immutable plan. Malformed input, schema drift, receipt
    /// tampering, and ownership collisions fail here, before any write.
    pub(crate) fn build(operation: CursorOperation, bin: &str) -> Result<Self> {
        let hooks_snapshot = FileSnapshot::capture(cursor_hooks_path())?;
        let mcp_snapshot = FileSnapshot::capture(cursor_mcp_path())?;
        let config_snapshot = FileSnapshot::capture(crate::runtime_config::config_path())?;

        let hooks_doc = parse_json_snapshot(&hooks_snapshot)?;
        if let Some(doc) = hooks_doc.as_ref() {
            validate_hooks_document(doc).map_err(|error| {
                anyhow::anyhow!(
                    "{}: cursor hooks schema error: {error}",
                    hooks_snapshot.path.display()
                )
            })?;
        }
        let mcp_doc = parse_json_snapshot(&mcp_snapshot)?;
        if let Some(doc) = mcp_doc.as_ref() {
            validate_mcp_document(doc).map_err(|error| {
                anyhow::anyhow!(
                    "{}: cursor mcp schema error: {error}",
                    mcp_snapshot.path.display()
                )
            })?;
        }

        let mut config_doc = match config_snapshot.bytes.as_ref() {
            Some(bytes) => std::str::from_utf8(bytes)
                .ok()
                .and_then(|text| text.parse::<DocumentMut>().ok())
                .with_context(|| {
                    format!(
                        "{} is not valid TOML (code=runtime_config_malformed)",
                        config_snapshot.path.display()
                    )
                })?,
            None => DocumentMut::new(),
        };
        let receipt = parse_receipt_from_config(&config_doc)?;

        let remem_entry = mcp_doc
            .as_ref()
            .and_then(|doc| doc.get("mcpServers"))
            .and_then(|servers| servers.get("remem"));
        let ownership = classify_remem_mcp(remem_entry, bin, receipt.as_ref());
        if ownership == McpOwnership::Collision {
            bail!(
                "{}: mcpServers.remem is occupied by an entry remem cannot prove it owns; refusing to modify it (code=ownership_collision)",
                mcp_snapshot.path.display()
            );
        }

        match operation {
            CursorOperation::Install { hooks_only } => Self::build_install(
                operation,
                hooks_only,
                bin,
                hooks_snapshot,
                mcp_snapshot,
                mcp_doc,
                ownership,
                config_snapshot,
                &mut config_doc,
            ),
            CursorOperation::Uninstall => Self::build_uninstall(
                operation,
                hooks_snapshot,
                hooks_doc,
                mcp_snapshot,
                mcp_doc,
                ownership,
                config_snapshot,
                &mut config_doc,
                receipt.as_ref(),
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_install(
        operation: CursorOperation,
        hooks_only: bool,
        bin: &str,
        hooks_snapshot: FileSnapshot,
        mcp_snapshot: FileSnapshot,
        mcp_doc: Option<Value>,
        ownership: McpOwnership,
        config_snapshot: FileSnapshot,
        config_doc: &mut DocumentMut,
    ) -> Result<Self> {
        // Contract v1 has no installable hook components (all capability
        // gates closed); hooks.json is validated and preserved verbatim.
        let hooks = FilePlan {
            snapshot: hooks_snapshot,
            action: FileAction::NoOp,
            reason: "validated; contract v1 installs no hook entries (capability gates closed)",
            new_bytes: None,
        };

        let mcp = if hooks_only {
            FilePlan {
                snapshot: mcp_snapshot,
                action: FileAction::NoOp,
                reason: "validated/no-change (hooks-only)",
                new_bytes: None,
            }
        } else {
            match ownership {
                McpOwnership::OwnedCurrent => FilePlan {
                    snapshot: mcp_snapshot,
                    action: FileAction::NoOp,
                    reason: "mcpServers.remem already matches the current managed entry",
                    new_bytes: None,
                },
                McpOwnership::Absent | McpOwnership::OwnedReceipt => {
                    let mut doc = mcp_doc.unwrap_or_else(|| json!({}));
                    let root = doc
                        .as_object_mut()
                        .expect("validated mcp document root must be an object");
                    let servers = root
                        .entry("mcpServers")
                        .or_insert_with(|| json!({}))
                        .as_object_mut()
                        .expect("validated mcpServers must be an object");
                    let action = if servers.contains_key("remem") {
                        FileAction::Replace
                    } else {
                        FileAction::Add
                    };
                    servers.insert("remem".to_string(), managed_cursor_mcp_entry(bin));
                    FilePlan {
                        new_bytes: Some(render_json_bytes(&doc)?),
                        snapshot: mcp_snapshot,
                        action,
                        reason: "write exact managed mcpServers.remem stdio entry",
                    }
                }
                McpOwnership::Collision => unreachable!("collision fails in build()"),
            }
        };

        plan_cursor_host_section(config_doc)?;
        let receipt = build_receipt(bin, hooks_only);
        let receipt_json =
            serde_json::to_string(&receipt).context("serialize cursor install receipt")?;
        set_cursor_receipt(config_doc, Some(&receipt_json))?;
        let config_bytes = config_doc.to_string().into_bytes();
        let runtime_config = FilePlan {
            action: if config_snapshot.bytes.as_deref() == Some(config_bytes.as_slice()) {
                FileAction::NoOp
            } else if config_snapshot.bytes.is_none() {
                FileAction::Add
            } else {
                FileAction::Replace
            },
            reason: "materialize hosts.cursor defaults and record the install receipt",
            new_bytes: if config_snapshot.bytes.as_deref() == Some(config_bytes.as_slice()) {
                None
            } else {
                Some(config_bytes)
            },
            snapshot: config_snapshot,
        };

        Ok(Self {
            operation,
            hooks,
            mcp,
            runtime_config,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_uninstall(
        operation: CursorOperation,
        hooks_snapshot: FileSnapshot,
        hooks_doc: Option<Value>,
        mcp_snapshot: FileSnapshot,
        mcp_doc: Option<Value>,
        ownership: McpOwnership,
        config_snapshot: FileSnapshot,
        config_doc: &mut DocumentMut,
        receipt: Option<&CursorInstallReceipt>,
    ) -> Result<Self> {
        // Remove exactly the receipt-recorded managed hook entries; every
        // other entry (including commands that merely contain "remem") is
        // foreign and preserved (B-007/B-017).
        let hooks = match (hooks_doc, receipt) {
            (Some(mut doc), Some(receipt)) => {
                let removed = remove_receipt_hook_entries(&mut doc, receipt);
                if removed {
                    FilePlan {
                        new_bytes: Some(render_json_bytes(&doc)?),
                        snapshot: hooks_snapshot,
                        action: FileAction::Remove,
                        reason: "remove receipt-recorded managed hook entries",
                    }
                } else {
                    FilePlan {
                        snapshot: hooks_snapshot,
                        action: FileAction::NoOp,
                        reason: "no managed hook entries recorded for this file",
                        new_bytes: None,
                    }
                }
            }
            (snapshot_doc, _) => FilePlan {
                snapshot: hooks_snapshot,
                action: FileAction::NoOp,
                reason: if snapshot_doc.is_some() {
                    "no managed hook entries recorded for this file"
                } else {
                    "file does not exist"
                },
                new_bytes: None,
            },
        };

        let mcp = match ownership {
            McpOwnership::Absent => FilePlan {
                snapshot: mcp_snapshot,
                action: FileAction::NoOp,
                reason: "no managed mcpServers.remem entry present",
                new_bytes: None,
            },
            McpOwnership::OwnedCurrent | McpOwnership::OwnedReceipt => {
                let mut doc = mcp_doc.expect("owned entry implies parsed document");
                if let Some(servers) = doc.get_mut("mcpServers").and_then(Value::as_object_mut) {
                    servers.remove("remem");
                }
                FilePlan {
                    new_bytes: Some(render_json_bytes(&doc)?),
                    snapshot: mcp_snapshot,
                    action: FileAction::Remove,
                    reason: "remove exactly the managed mcpServers.remem entry",
                }
            }
            McpOwnership::Collision => unreachable!("collision fails in build()"),
        };

        let runtime_config = if receipt.is_some() {
            set_cursor_receipt(config_doc, None)?;
            let config_bytes = config_doc.to_string().into_bytes();
            FilePlan {
                action: FileAction::Remove,
                reason: "clear the managed cursor install receipt",
                new_bytes: Some(config_bytes),
                snapshot: config_snapshot,
            }
        } else {
            FilePlan {
                snapshot: config_snapshot,
                action: FileAction::NoOp,
                reason: "no cursor install receipt recorded",
                new_bytes: None,
            }
        };

        Ok(Self {
            operation,
            hooks,
            mcp,
            runtime_config,
        })
    }

    /// Renders the B-016 dry-run lines: both absolute paths plus the action
    /// and a non-sensitive reason. Never touches disk.
    pub(crate) fn dry_run_lines(&self) -> Vec<String> {
        let header = match self.operation {
            CursorOperation::Install { hooks_only: true } => "  plan: install (hooks-only)",
            CursorOperation::Install { hooks_only: false } => "  plan: install",
            CursorOperation::Uninstall => "  plan: uninstall",
        };
        std::iter::once(header.to_string())
            .chain(
                [
                    ("hooks ", &self.hooks),
                    ("MCP   ", &self.mcp),
                    ("config", &self.runtime_config),
                ]
                .into_iter()
                .map(|(label, plan)| {
                    format!(
                        "  {label} -> {} [{}] {}",
                        plan.snapshot.path.display(),
                        plan.action.label(),
                        plan.reason
                    )
                }),
            )
            .collect()
    }
}

fn set_cursor_receipt(doc: &mut DocumentMut, receipt_json: Option<&str>) -> Result<()> {
    let memory_ai = doc
        .entry("memory_ai")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("memory_ai exists but is not a table (code=runtime_config_malformed)")?;
    let hosts = memory_ai
        .entry("hosts")
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context("memory_ai.hosts exists but is not a table (code=runtime_config_malformed)")?;
    let table = hosts
        .entry(crate::runtime_config::CURSOR_HOST)
        .or_insert_with(|| Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .context(
            "memory_ai.hosts.cursor exists but is not a table (code=runtime_config_malformed)",
        )?;
    match receipt_json {
        Some(receipt_json) => table["install_receipt"] = value(receipt_json),
        None => {
            table.remove("install_receipt");
        }
    }
    Ok(())
}

/// Removes hook entries that exactly match a receipt-recorded hook component
/// (event, rendered command, timeout, and whole-entry digest). Returns true
/// when at least one entry was removed.
fn remove_receipt_hook_entries(doc: &mut Value, receipt: &CursorInstallReceipt) -> bool {
    let mut removed = false;
    let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) else {
        return false;
    };
    for component in receipt.components.iter().filter(|c| c.kind == "hook") {
        let Some(entries) = hooks.get_mut(&component.key).and_then(Value::as_array_mut) else {
            continue;
        };
        entries.retain(|entry| {
            let matches_receipt = entry
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| Some(command) == component.rendered_command.as_deref())
                && entry.get("timeout").and_then(Value::as_i64) == component.timeout
                && canonical_json_digest(entry) == component.digest;
            if matches_receipt {
                removed = true;
            }
            !matches_receipt
        });
    }
    removed
}
