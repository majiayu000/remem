use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::install::config::{build_hooks, remove_remem_hooks, strip_hooks_json, HookStrategy};
use crate::install::host::{HookSupport, InstallHost};
use crate::install::json_io::{read_json_file, write_json_file};
use crate::install::paths::{codex_config_path, codex_hooks_path};

pub(in crate::install) struct CodexHost;

const SERVER_KEY: &str = "remem";

impl InstallHost for CodexHost {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn config_path(&self) -> PathBuf {
        codex_config_path()
    }

    fn is_available(&self) -> bool {
        codex_config_path().exists()
            || dirs::home_dir()
                .map(|h| h.join(".codex").exists())
                .unwrap_or(false)
    }

    fn install_mcp(&self, bin: &str) -> Result<()> {
        let path = codex_config_path();
        let mut doc = read_toml_doc(&path)?;
        upsert_remem_server(&mut doc, bin)?;
        enable_codex_hooks(&mut doc)?;
        write_toml_doc(&path, &doc)?;
        Ok(())
    }

    fn uninstall_mcp(&self, bin: &str) -> Result<()> {
        let path = codex_config_path();
        if !path.exists() {
            return Ok(());
        }
        let mut doc = read_toml_doc(&path)?;
        remove_remem_server(&mut doc, bin);
        write_toml_doc(&path, &doc)?;
        Ok(())
    }

    fn install_hooks(&self, bin: &str) -> Result<HookSupport> {
        apply_codex_hooks_json(&codex_hooks_path(), bin)?;
        Ok(HookSupport::Installed)
    }

    fn uninstall_hooks(&self, bin: &str) -> Result<()> {
        strip_hooks_json(&codex_hooks_path(), bin)
    }

    fn dry_run_plan(&self, bin: &str) -> Vec<String> {
        vec![
            format!(
                "  MCP    -> {} (add [mcp_servers.{}])",
                codex_config_path().display(),
                SERVER_KEY
            ),
            format!(
                "  hooks  -> {} (SessionStart/PostToolUse(Bash)/Stop)",
                codex_hooks_path().display()
            ),
            format!("  binary -> {}", bin),
        ]
    }
}

// --- TOML helpers ---------------------------------------------------------

fn read_toml_doc(path: &Path) -> Result<DocumentMut> {
    if !path.exists() {
        return Ok(DocumentMut::new());
    }
    let content =
        std::fs::read_to_string(path).with_context(|| format!("读取 {} 失败", path.display()))?;
    content
        .parse::<DocumentMut>()
        .with_context(|| format!("解析 {} 失败（非法 TOML）", path.display()))
}

fn write_toml_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录 {} 失败", parent.display()))?;
    }
    std::fs::write(path, doc.to_string())
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(())
}

fn apply_codex_hooks_json(path: &Path, bin: &str) -> Result<()> {
    let mut doc = read_json_file(&path.to_path_buf())?;
    remove_remem_hooks(&mut doc, bin);

    let new_hooks = build_codex_hooks(bin);
    let obj = doc
        .as_object_mut()
        .with_context(|| format!("{} 根节点不是 Object", path.display()))?;
    let hooks = obj
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if let (Some(existing), Some(new)) = (hooks.as_object_mut(), new_hooks.as_object()) {
        for (event_type, entries) in new {
            let arr = existing
                .entry(event_type.to_string())
                .or_insert_with(|| serde_json::json!([]));
            if let (Some(arr), Some(new_entries)) = (arr.as_array_mut(), entries.as_array()) {
                for entry in new_entries {
                    arr.push(entry.clone());
                }
            }
        }
    }
    write_json_file(&path.to_path_buf(), &doc)
}

fn build_codex_hooks(bin: &str) -> Value {
    let mut hooks = build_hooks(bin, HookStrategy::Codex);
    convert_hook_timeouts_to_seconds(&mut hooks);
    hooks
}

fn convert_hook_timeouts_to_seconds(value: &mut Value) {
    let Some(events) = value.as_object_mut() else {
        return;
    };

    for entries in events.values_mut() {
        let Some(entries) = entries.as_array_mut() else {
            continue;
        };
        for entry in entries {
            let Some(hooks) = entry
                .get_mut("hooks")
                .and_then(|hooks| hooks.as_array_mut())
            else {
                continue;
            };
            for hook in hooks {
                let Some(timeout) = hook.get_mut("timeout") else {
                    continue;
                };
                let Some(ms) = timeout.as_i64() else {
                    continue;
                };
                *timeout = Value::from((ms / 1000).max(1));
            }
        }
    }
}

fn enable_codex_hooks(doc: &mut DocumentMut) -> Result<()> {
    let features = doc
        .entry("features")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .context("features 存在但不是 table，拒绝覆盖")?;
    features["codex_hooks"] = value(true);
    Ok(())
}

/// Ensure `[mcp_servers.remem]` points at the current binary.
///
/// Preserves any sibling `[mcp_servers.*]` entries (e.g. omx_state) plus
/// surrounding comments/formatting. Returns an error if the config has
/// a non-table value at `mcp_servers` (e.g. a user typo like
/// `mcp_servers = "foo"`), since we won't silently clobber that.
fn upsert_remem_server(doc: &mut DocumentMut, bin: &str) -> Result<()> {
    // Also clean up any non-canonical entry pointing at a stale remem binary.
    remove_remem_server(doc, bin);

    let servers = doc
        .entry("mcp_servers")
        .or_insert(Item::Table({
            let mut t = Table::new();
            t.set_implicit(true);
            t
        }))
        .as_table_mut()
        .context("mcp_servers 存在但不是 table，拒绝覆盖")?;

    let mut entry = Table::new();
    entry["command"] = value(bin);
    let mut args = Array::new();
    args.push("mcp");
    entry["args"] = value(args);
    entry["enabled"] = value(true);
    servers.insert(SERVER_KEY, Item::Table(entry));
    Ok(())
}

/// Remove any `[mcp_servers.*]` whose key is "remem" or whose `command`
/// matches the given binary path (defensively catches renamed stale entries).
fn remove_remem_server(doc: &mut DocumentMut, bin: &str) {
    let Some(servers) = doc
        .get_mut("mcp_servers")
        .and_then(|item| item.as_table_mut())
    else {
        return;
    };

    let keys: Vec<String> = servers.iter().map(|(k, _)| k.to_string()).collect();
    for key in keys {
        if key == SERVER_KEY {
            servers.remove(&key);
            continue;
        }
        let cmd_matches = servers
            .get(&key)
            .and_then(|item| item.as_table())
            .and_then(|t| t.get("command"))
            .and_then(|v| v.as_str())
            .map(|s| s == bin || s.ends_with("/remem"))
            .unwrap_or(false);
        if cmd_matches {
            servers.remove(&key);
        }
    }

    if servers.is_empty() {
        doc.remove("mcp_servers");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_into_empty_doc() {
        let mut doc = DocumentMut::new();
        upsert_remem_server(&mut doc, "/tmp/remem").unwrap();
        let rendered = doc.to_string();
        assert!(rendered.contains("[mcp_servers.remem]"), "{rendered}");
        assert!(rendered.contains("command = \"/tmp/remem\""), "{rendered}");
        assert!(rendered.contains("args = [\"mcp\"]"), "{rendered}");
        assert!(rendered.contains("enabled = true"), "{rendered}");
    }

    #[test]
    fn upsert_preserves_other_servers_and_comments() {
        let src = r#"# Top-level comment
[mcp_servers.omx_state]
command = "node"
args = ["state-server.js"]
enabled = true
startup_timeout_sec = 5
"#;
        let mut doc: DocumentMut = src.parse().unwrap();
        upsert_remem_server(&mut doc, "/tmp/remem").unwrap();
        let rendered = doc.to_string();
        assert!(rendered.contains("# Top-level comment"), "{rendered}");
        assert!(rendered.contains("[mcp_servers.omx_state]"), "{rendered}");
        assert!(rendered.contains("startup_timeout_sec = 5"), "{rendered}");
        assert!(rendered.contains("[mcp_servers.remem]"), "{rendered}");
    }

    #[test]
    fn enable_codex_hooks_adds_feature_flag() {
        let mut doc = DocumentMut::new();
        enable_codex_hooks(&mut doc).unwrap();
        let rendered = doc.to_string();
        assert!(rendered.contains("[features]"), "{rendered}");
        assert!(rendered.contains("codex_hooks = true"), "{rendered}");
    }

    #[test]
    fn build_codex_hooks_uses_second_timeouts() {
        let hooks = build_codex_hooks("/tmp/remem");
        assert_eq!(hooks["SessionStart"][0]["hooks"][0]["timeout"], 15);
        assert_eq!(hooks["PostToolUse"][0]["hooks"][0]["timeout"], 3);
        assert_eq!(hooks["PostToolUse"][0]["matcher"], "Bash");
        assert_eq!(
            hooks["PostToolUse"][0]["hooks"][0]["command"],
            "REMEM_HOOK_ADAPTER=codex-cli /tmp/remem observe"
        );
        assert_eq!(hooks["Stop"][0]["hooks"][0]["timeout"], 120);
        assert!(hooks.get("UserPromptSubmit").is_none());
        assert_eq!(
            hooks["Stop"][0]["hooks"][0]["command"],
            "REMEM_SUMMARY_EXECUTOR=codex-cli /tmp/remem summarize"
        );
    }

    #[test]
    fn upsert_is_idempotent() {
        let mut doc = DocumentMut::new();
        upsert_remem_server(&mut doc, "/tmp/remem").unwrap();
        let first = doc.to_string();
        upsert_remem_server(&mut doc, "/tmp/remem").unwrap();
        let second = doc.to_string();
        assert_eq!(first, second);
    }

    #[test]
    fn upsert_updates_binary_path() {
        let mut doc = DocumentMut::new();
        upsert_remem_server(&mut doc, "/old/remem").unwrap();
        upsert_remem_server(&mut doc, "/new/remem").unwrap();
        let rendered = doc.to_string();
        assert!(rendered.contains("/new/remem"), "{rendered}");
        assert!(!rendered.contains("/old/remem"), "{rendered}");
    }

    #[test]
    fn upsert_errors_when_mcp_servers_is_not_a_table() {
        let src = r#"mcp_servers = "not-a-table"
"#;
        let mut doc: DocumentMut = src.parse().unwrap();
        let err = upsert_remem_server(&mut doc, "/tmp/remem").unwrap_err();
        assert!(err.to_string().contains("不是 table"), "{err}");
    }

    #[test]
    fn remove_leaves_other_servers_intact() {
        let src = r#"[mcp_servers.omx_state]
command = "node"
args = ["state-server.js"]
enabled = true

[mcp_servers.remem]
command = "/tmp/remem"
args = ["mcp"]
enabled = true
"#;
        let mut doc: DocumentMut = src.parse().unwrap();
        remove_remem_server(&mut doc, "/tmp/remem");
        let rendered = doc.to_string();
        assert!(rendered.contains("[mcp_servers.omx_state]"), "{rendered}");
        assert!(!rendered.contains("[mcp_servers.remem]"), "{rendered}");
    }

    #[test]
    fn remove_when_section_absent_is_noop() {
        let src = "[other_section]\nkey = 1\n";
        let mut doc: DocumentMut = src.parse().unwrap();
        remove_remem_server(&mut doc, "/tmp/remem");
        assert!(doc.to_string().contains("[other_section]"));
    }

    #[test]
    fn remove_cleans_empty_section() {
        let src = r#"[mcp_servers.remem]
command = "/tmp/remem"
args = ["mcp"]
enabled = true
"#;
        let mut doc: DocumentMut = src.parse().unwrap();
        remove_remem_server(&mut doc, "/tmp/remem");
        let rendered = doc.to_string();
        assert!(!rendered.contains("mcp_servers"), "{rendered}");
    }
}
