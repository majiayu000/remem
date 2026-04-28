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
    known_hosts().into_iter().filter(host_present).collect()
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

    let events = expected_hook_events(probe.name);
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

fn display_mcp_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(" or ")
}

fn expected_hook_events(host: &str) -> &'static [&'static str] {
    match host {
        "codex" => &["SessionStart", "PostToolUse", "Stop"],
        _ => &["PostToolUse", "Stop", "SessionStart", "UserPromptSubmit"],
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
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn temp_path(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("remem-{label}-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct HomeGuard {
        previous: Option<OsString>,
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(previous) => unsafe { std::env::set_var("HOME", previous) },
                None => unsafe { std::env::remove_var("HOME") },
            }
        }
    }

    fn with_home_dir<T>(home: &PathBuf, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock should acquire");
        let _home_guard = HomeGuard {
            previous: std::env::var_os("HOME"),
        };
        unsafe { std::env::set_var("HOME", home) };
        f()
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
        assert!(check.detail.contains("1/3 registered"), "{}", check.detail);
    }

    #[test]
    fn probe_hooks_accepts_codex_strategy() {
        let dir = temp_path("doctor-codex-hooks");
        let hooks_path = dir.join("hooks.json");
        std::fs::write(
            &hooks_path,
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context" }] }],
    "PostToolUse": [{ "hooks": [{ "command": "REMEM_HOOK_ADAPTER=codex-cli /tmp/remem observe" }] }],
    "Stop": [{ "hooks": [{ "command": "REMEM_SUMMARY_EXECUTOR=codex-cli /tmp/remem summarize" }] }]
  }
}"#,
        )
        .unwrap();

        let check = probe_hooks(HostProbe {
            name: "codex",
            hooks_path,
            mcp_paths: vec![dir.join("config.toml")],
        });

        assert!(matches!(check.status, Status::Ok));
        assert!(check.detail.contains("3/3 registered"), "{}", check.detail);
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
    fn active_hosts_keeps_all_present_hosts() {
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

        let expected = vec![claude.clone(), codex.clone()];
        let hosts: Vec<_> = expected.clone().into_iter().filter(host_present).collect();

        assert_eq!(hosts, expected);
    }

    #[test]
    fn doctor_reports_each_present_host_even_if_only_one_targets_remem() {
        let home = temp_path("doctor-home");
        let claude_dir = home.join(".claude");
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::create_dir_all(&codex_dir).unwrap();

        std::fs::write(
            codex_dir.join("hooks.json"),
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context" }] }],
    "PostToolUse": [{ "hooks": [{ "command": "/tmp/remem observe" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize" }] }]
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            r#"[mcp_servers.remem]
command = "/tmp/remem"
"#,
        )
        .unwrap();
        std::fs::write(
            home.join(".claude.json"),
            r#"{ "mcpServers": { "other": {} } }"#,
        )
        .unwrap();

        with_home_dir(&home, || {
            let hook_checks = check_hooks();
            assert_eq!(hook_checks.len(), 2);
            assert_eq!(hook_checks[0].name, "Hooks (claude)");
            assert!(matches!(hook_checks[0].status, Status::Fail));
            assert_eq!(hook_checks[1].name, "Hooks (codex)");
            assert!(matches!(hook_checks[1].status, Status::Ok));

            let mcp_checks = check_mcp();
            assert_eq!(mcp_checks.len(), 2);
            assert_eq!(mcp_checks[0].name, "MCP (claude)");
            assert!(matches!(mcp_checks[0].status, Status::Fail));
            assert_eq!(mcp_checks[1].name, "MCP (codex)");
            assert!(matches!(mcp_checks[1].status, Status::Ok));
        });
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
