use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::install::config::{apply_hooks_json, strip_hooks_json};
use crate::install::host::{HookSupport, InstallHost};
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
        // Codex's ~/.codex/hooks.json uses the same schema as Claude's
        // settings.json (SessionStart / UserPromptSubmit / PreToolUse /
        // PostToolUse / Stop, stdin JSON events). Reuse the shared merger.
        apply_hooks_json(&codex_hooks_path(), bin)?;
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
                "  hooks  -> {} (SessionStart/UserPromptSubmit/PostToolUse/Stop)",
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
