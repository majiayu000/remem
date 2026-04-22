use serde_json::Value;
use std::path::PathBuf;
use toml_edit::DocumentMut;

use super::types::{Check, Status};

pub(super) fn check_binary() -> Check {
    let exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    Check {
        name: "Binary",
        status: Status::Ok,
        detail: exe,
    }
}

/// A single host we know how to validate. The strings are leaked static
/// because `Check::name` takes `&'static str` — every host lives for the
/// process, so leaking is fine.
#[derive(Clone, Debug, PartialEq, Eq)]
struct HostProbe {
    name: &'static str,
    hooks_path: PathBuf,
    mcp_paths: Vec<PathBuf>,
}

fn known_hosts() -> Vec<HostProbe> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    vec![
        HostProbe {
            name: "claude",
            hooks_path: home.join(".claude").join("settings.json"),
            mcp_paths: vec![
                home.join(".claude.json"),
                home.join(".claude").join("claude_desktop_config.json"),
            ],
        },
        HostProbe {
            name: "codex",
            hooks_path: home.join(".codex").join("hooks.json"),
            mcp_paths: vec![home.join(".codex").join("config.toml")],
        },
    ]
}

/// True if the host's config directory exists — i.e. the tool is installed
/// on this machine and worth probing.
fn host_present(probe: &HostProbe) -> bool {
    probe.hooks_path.parent().is_some_and(|p| p.exists())
        || probe.hooks_path.exists()
        || probe.mcp_paths.iter().any(|path| path.exists())
}

fn active_hosts() -> Vec<HostProbe> {
    let hosts: Vec<HostProbe> = known_hosts().into_iter().filter(host_present).collect();
    if hosts.is_empty() {
        return hosts;
    }

    let targeted: Vec<HostProbe> = hosts
        .iter()
        .filter(|probe| host_targets_remem(probe))
        .cloned()
        .collect();
    if targeted.is_empty() {
        hosts
    } else {
        targeted
    }
}

/// Produce one Check per detected host's hooks file. Hosts whose config
/// directory doesn't exist are silently skipped — they aren't installed, so
/// there's nothing to validate.
pub(super) fn check_hooks() -> Vec<Check> {
    let mut checks = Vec::new();
    for probe in active_hosts() {
        checks.push(probe_hooks(probe));
    }
    if checks.is_empty() {
        checks.push(Check {
            name: "Hooks",
            status: Status::Fail,
            detail: "no supported host detected (install Claude Code or Codex)".to_string(),
        });
    }
    checks
}

pub(super) fn check_mcp() -> Vec<Check> {
    let mut checks = Vec::new();
    for probe in active_hosts() {
        checks.push(probe_mcp(probe));
    }
    if checks.is_empty() {
        checks.push(Check {
            name: "MCP server",
            status: Status::Fail,
            detail: "no supported host detected".to_string(),
        });
    }
    checks
}

fn probe_hooks(probe: HostProbe) -> Check {
    let name = hooks_check_name(probe.name);

    if !probe.hooks_path.exists() {
        return Check {
            name,
            status: Status::Fail,
            detail: format!(
                "{} not found (run `remem install`)",
                probe.hooks_path.display()
            ),
        };
    }

    let content = match std::fs::read_to_string(&probe.hooks_path) {
        Ok(content) => content,
        Err(err) => {
            return Check {
                name,
                status: Status::Fail,
                detail: format!("cannot read {}: {}", probe.hooks_path.display(), err),
            };
        }
    };

    let doc: Value = match serde_json::from_str(&content) {
        Ok(doc) => doc,
        Err(err) => {
            return Check {
                name,
                status: Status::Fail,
                detail: format!("cannot parse {}: {}", probe.hooks_path.display(), err),
            };
        }
    };

    let events = ["PostToolUse", "Stop", "SessionStart", "UserPromptSubmit"];
    let found = events
        .iter()
        .filter(|event| event_has_remem_hook(&doc, event))
        .count();

    if found == events.len() {
        Check {
            name,
            status: Status::Ok,
            detail: format!(
                "{}/{} registered in {}",
                found,
                events.len(),
                probe.hooks_path.display()
            ),
        }
    } else if found > 0 {
        Check {
            name,
            status: Status::Warn,
            detail: format!(
                "{}/{} registered (run `remem install --target {}` to fix)",
                found,
                events.len(),
                probe.name
            ),
        }
    } else {
        Check {
            name,
            status: Status::Fail,
            detail: format!(
                "no remem hooks (run `remem install --target {}`)",
                probe.name
            ),
        }
    }
}

fn probe_mcp(probe: HostProbe) -> Check {
    let name = mcp_check_name(probe.name);
    let has_existing_path = probe.mcp_paths.iter().any(|path| path.exists());
    if let Some(result) = probe
        .mcp_paths
        .iter()
        .filter(|path| path.exists())
        .find_map(|path| probe_mcp_path(probe.name, path))
    {
        return match result {
            Ok(path) => Check {
                name,
                status: Status::Ok,
                detail: format!("registered in {}", path.display()),
            },
            Err((path, err)) => Check {
                name,
                status: Status::Fail,
                detail: format!("cannot parse {}: {}", path.display(), err),
            },
        };
    }

    Check {
        name,
        status: Status::Fail,
        detail: if has_existing_path {
            format!(
                "not registered (run `remem install --target {}`)",
                probe.name
            )
        } else {
            format!(
                "{} not found (run `remem install --target {}`)",
                display_mcp_paths(&probe.mcp_paths),
                probe.name
            )
        },
    }
}

fn hooks_check_name(host: &str) -> &'static str {
    match host {
        "claude" => "Hooks (claude)",
        "codex" => "Hooks (codex)",
        _ => "Hooks",
    }
}

fn mcp_check_name(host: &str) -> &'static str {
    match host {
        "claude" => "MCP (claude)",
        "codex" => "MCP (codex)",
        _ => "MCP server",
    }
}

fn host_targets_remem(probe: &HostProbe) -> bool {
    hooks_file_has_remem(&probe.hooks_path) || mcp_file_has_remem(probe)
}

fn display_mcp_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" or ")
}

fn hooks_file_has_remem(path: &PathBuf) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    match serde_json::from_str::<Value>(&content) {
        Ok(doc) => ["PostToolUse", "Stop", "SessionStart", "UserPromptSubmit"]
            .iter()
            .any(|event| event_has_remem_hook(&doc, event)),
        Err(_) => content.contains("remem"),
    }
}

fn mcp_file_has_remem(probe: &HostProbe) -> bool {
    probe
        .mcp_paths
        .iter()
        .any(|path| path_has_remem_mcp(probe.name, path))
}

fn path_has_remem_mcp(host: &str, path: &PathBuf) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    match host {
        "claude" => match serde_json::from_str::<Value>(&content) {
            Ok(doc) => claude_has_remem_mcp(&doc),
            Err(_) => content.contains("remem"),
        },
        "codex" => match content.parse::<DocumentMut>() {
            Ok(doc) => codex_has_remem_mcp(&doc),
            Err(_) => content.contains("remem"),
        },
        _ => false,
    }
}

fn probe_mcp_path<'a>(
    host: &str,
    path: &'a PathBuf,
) -> Option<Result<&'a PathBuf, (&'a PathBuf, String)>> {
    let content = std::fs::read_to_string(path).ok()?;
    let has_remem = match host {
        "claude" => match serde_json::from_str::<Value>(&content) {
            Ok(doc) => claude_has_remem_mcp(&doc),
            Err(err) => return Some(Err((path, err.to_string()))),
        },
        "codex" => match content.parse::<DocumentMut>() {
            Ok(doc) => codex_has_remem_mcp(&doc),
            Err(err) => return Some(Err((path, err.to_string()))),
        },
        _ => false,
    };
    if has_remem {
        Some(Ok(path))
    } else {
        None
    }
}

fn event_has_remem_hook(doc: &Value, event: &str) -> bool {
    doc.get("hooks")
        .and_then(|hooks| hooks.get(event))
        .and_then(|entries| entries.as_array())
        .into_iter()
        .flatten()
        .any(entry_has_remem_hook)
}

fn entry_has_remem_hook(entry: &Value) -> bool {
    entry
        .get("hooks")
        .and_then(|hooks| hooks.as_array())
        .into_iter()
        .flatten()
        .any(|hook| {
            hook.get("command")
                .and_then(|command| command.as_str())
                .is_some_and(|command| command.contains("remem"))
        })
}

fn claude_has_remem_mcp(doc: &Value) -> bool {
    doc.get("mcpServers")
        .and_then(|servers| servers.as_object())
        .is_some_and(|servers| servers.contains_key("remem"))
}

fn codex_has_remem_mcp(doc: &DocumentMut) -> bool {
    doc.get("mcp_servers")
        .and_then(|servers| servers.as_table())
        .is_some_and(|servers| servers.contains_key("remem"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    fn temp_path(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("remem-{label}-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn probe_hooks_requires_remem_on_each_event() {
        let dir = temp_path("doctor-hooks");
        let hooks_path = dir.join("hooks.json");
        std::fs::write(
            &hooks_path,
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context" }] }],
    "Stop": [{ "hooks": [{ "command": "other-tool summarize" }] }],
    "PostToolUse": [{ "hooks": [{ "command": "other-tool observe" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "command": "other-tool init" }] }]
  }
}"#,
        )
        .unwrap();

        let check = probe_hooks(HostProbe {
            name: "codex",
            hooks_path,
            mcp_paths: vec![dir.join("config.toml")],
        });

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("1/4 registered"), "{}", check.detail);
    }

    #[test]
    fn probe_mcp_requires_exact_codex_remem_entry() {
        let dir = temp_path("doctor-mcp");
        let mcp_path = dir.join("config.toml");
        std::fs::write(
            &mcp_path,
            r#"# remem should not be detected from comments
[mcp_servers.other]
command = "echo"
note = "remem"
"#,
        )
        .unwrap();

        let check = probe_mcp(HostProbe {
            name: "codex",
            hooks_path: dir.join("hooks.json"),
            mcp_paths: vec![mcp_path],
        });

        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("not registered"), "{}", check.detail);
    }

    #[test]
    fn active_hosts_prefers_hosts_with_remem_markers() {
        let claude_dir = temp_path("doctor-claude");
        let claude = HostProbe {
            name: "claude",
            hooks_path: claude_dir.join("settings.json"),
            mcp_paths: vec![claude_dir.join("claude.json")],
        };
        std::fs::write(&claude.mcp_paths[0], r#"{ "mcpServers": { "other": {} } }"#).unwrap();

        let codex_dir = temp_path("doctor-codex");
        let codex = HostProbe {
            name: "codex",
            hooks_path: codex_dir.join("hooks.json"),
            mcp_paths: vec![codex_dir.join("config.toml")],
        };
        std::fs::write(
            &codex.mcp_paths[0],
            r#"[mcp_servers.remem]
command = "/tmp/remem"
"#,
        )
        .unwrap();

        let hosts = {
            let present = vec![claude, codex.clone()];
            let targeted: Vec<_> = present
                .iter()
                .filter(|probe| host_targets_remem(probe))
                .cloned()
                .collect();
            if targeted.is_empty() {
                present
            } else {
                targeted
            }
        };

        assert_eq!(hosts, vec![codex]);
    }

    #[test]
    fn probe_mcp_accepts_claude_desktop_config_path() {
        let dir = temp_path("doctor-claude-desktop");
        std::fs::write(
            dir.join("claude_desktop_config.json"),
            r#"{ "mcpServers": { "remem": { "command": "/tmp/remem" } } }"#,
        )
        .unwrap();

        let check = probe_mcp(HostProbe {
            name: "claude",
            hooks_path: dir.join("settings.json"),
            mcp_paths: vec![
                dir.join("claude.json"),
                dir.join("claude_desktop_config.json"),
            ],
        });

        assert!(matches!(check.status, Status::Ok));
        assert!(
            check.detail.contains("claude_desktop_config.json"),
            "{}",
            check.detail
        );
    }
}
