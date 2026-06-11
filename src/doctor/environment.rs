use serde_json::Value;
use std::path::PathBuf;
use toml_edit::DocumentMut;

use super::hook_validation::{
    event_has_expected_remem_hook, event_has_remem_subcommand_hook, expected_hook_command,
    expected_hook_events, expected_hook_executable_from_hooks, extract_remem_command_path,
    hook_command_strings,
};
use super::types::{Check, Status};

pub(super) fn check_binary() -> Check {
    let exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    Check::new("Binary", Status::Ok, exe)
}

pub(super) fn check_install_paths() -> Check {
    let mut configured = configured_remem_paths_for(active_hosts());
    if configured.is_empty() {
        configured.extend(
            std::env::var_os("REMEM_INSTALL_BINARY")
                .map(PathBuf::from)
                .or_else(|| std::env::current_exe().ok()),
        );
    }
    let report =
        crate::install::duplicates::inspect_install_paths_with_configured_paths(&configured);
    Check::new(
        "Install paths",
        if report.has_warning() {
            Status::Warn
        } else {
            Status::Ok
        },
        crate::install::duplicates::format_doctor_detail(&report),
    )
}

fn configured_remem_paths_for(hosts: Vec<HostProbe>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for probe in hosts {
        paths.extend(configured_hook_paths(&probe.hooks_path));
        paths.extend(configured_mcp_paths(&probe));
    }
    dedupe_paths(paths)
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
    check_hooks_for(active_hosts())
}

fn check_hooks_for(hosts: Vec<HostProbe>) -> Vec<Check> {
    let mut checks = Vec::new();
    for probe in hosts {
        checks.push(probe_hooks(probe));
    }
    if checks.is_empty() {
        checks.push(Check::new(
            "Hooks",
            Status::Fail,
            "no supported host detected (install Claude Code or Codex)",
        ));
    }
    checks
}

pub(super) fn check_mcp() -> Vec<Check> {
    check_mcp_for(active_hosts())
}

fn check_mcp_for(hosts: Vec<HostProbe>) -> Vec<Check> {
    let mut checks = Vec::new();
    for probe in hosts {
        checks.push(probe_mcp(probe));
    }
    if checks.is_empty() {
        checks.push(Check::new(
            "MCP server",
            Status::Fail,
            "no supported host detected",
        ));
    }
    checks
}

fn probe_hooks(probe: HostProbe) -> Check {
    let name = hooks_check_name(probe.name);

    if !probe.hooks_path.exists() {
        return Check::new(
            name,
            Status::Fail,
            format!(
                "{} not found (run `remem install`)",
                probe.hooks_path.display()
            ),
        );
    }

    let content = match std::fs::read_to_string(&probe.hooks_path) {
        Ok(content) => content,
        Err(err) => {
            return Check::new(
                name,
                Status::Fail,
                format!("cannot read {}: {}", probe.hooks_path.display(), err),
            );
        }
    };

    let doc: Value = match serde_json::from_str(&content) {
        Ok(doc) => doc,
        Err(err) => {
            return Check::new(
                name,
                Status::Fail,
                format!("cannot parse {}: {}", probe.hooks_path.display(), err),
            );
        }
    };

    let events = expected_hook_events(probe.name);
    let expected_executable = expected_hook_executable(&doc, &probe);
    let found = events
        .iter()
        .filter(|event| {
            expected_executable
                .as_deref()
                .and_then(|executable| expected_hook_command(probe.name, event, executable))
                .is_some_and(|expected| event_has_expected_remem_hook(&doc, event, expected))
        })
        .count();
    let deprecated_codex_observe =
        probe.name == "codex" && event_has_remem_subcommand_hook(&doc, "PostToolUse", "observe");
    let legacy_policy = has_legacy_hook_policy(&doc);

    if found == events.len() {
        if legacy_policy {
            return Check::new(
                name,
                Status::Warn,
                format!(
                    "{}/{} registered in {}; legacy memory-AI hook policy remains (run `remem install --target {}`)",
                    found, events.len(), probe.hooks_path.display(), probe.name
                ),
            );
        }
        if deprecated_codex_observe {
            return Check::new(
                name,
                Status::Warn,
                format!(
                    "{}/{} registered in {}; remove Codex PostToolUse observe to avoid unbounded Bash backlog",
                    found,
                    events.len(),
                    probe.hooks_path.display()
                ),
            );
        }
        Check::new(
            name,
            Status::Ok,
            format!(
                "{}/{} registered in {}",
                found,
                events.len(),
                probe.hooks_path.display()
            ),
        )
    } else if found > 0 {
        Check::new(
            name,
            Status::Warn,
            format!(
                "{}/{} registered (run `remem install --target {}` to fix)",
                found,
                events.len(),
                probe.name
            ),
        )
    } else {
        Check::new(
            name,
            Status::Fail,
            format!(
                "no remem hooks (run `remem install --target {}`)",
                probe.name
            ),
        )
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
            Ok(path) => Check::new(
                name,
                Status::Ok,
                format!("registered in {}", path.display()),
            ),
            Err((path, err)) => Check::new(
                name,
                Status::Fail,
                format!("cannot parse {}: {}", path.display(), err),
            ),
        };
    }

    Check::new(
        name,
        Status::Fail,
        if has_existing_path {
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
    )
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

fn configured_mcp_paths(probe: &HostProbe) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for path in probe.mcp_paths.iter().filter(|path| path.exists()) {
        let Some(content) = std::fs::read_to_string(path).ok() else {
            continue;
        };
        match probe.name {
            "claude" => {
                if let Ok(doc) = serde_json::from_str::<Value>(&content) {
                    paths.extend(claude_remem_mcp_command(&doc).map(PathBuf::from));
                }
            }
            "codex" => {
                if let Ok(doc) = content.parse::<DocumentMut>() {
                    paths.extend(codex_remem_mcp_command(&doc).map(PathBuf::from));
                }
            }
            _ => {}
        }
    }
    paths
}

fn expected_hook_executable(doc: &Value, probe: &HostProbe) -> Option<PathBuf> {
    expected_hook_executable_from_hooks(doc, probe.name)
        .map(PathBuf::from)
        .or_else(|| configured_mcp_paths(probe).into_iter().next())
}

fn configured_hook_paths(path: &PathBuf) -> Vec<PathBuf> {
    let Some(content) = std::fs::read_to_string(path).ok() else {
        return Vec::new();
    };
    let Ok(doc) = serde_json::from_str::<Value>(&content) else {
        return Vec::new();
    };
    hook_command_strings(&doc)
        .filter_map(extract_remem_command_path)
        .map(PathBuf::from)
        .collect()
}

fn claude_remem_mcp_command(doc: &Value) -> Option<&str> {
    doc.get("mcpServers")
        .and_then(|servers| servers.get("remem"))
        .and_then(|server| server.get("command"))
        .and_then(|command| command.as_str())
}

fn codex_remem_mcp_command(doc: &DocumentMut) -> Option<&str> {
    doc.get("mcp_servers")
        .and_then(|servers| servers.as_table())
        .and_then(|servers| servers.get("remem"))
        .and_then(|server| server.as_table())
        .and_then(|server| server.get("command"))
        .and_then(|command| command.as_str())
}

fn dedupe_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn has_legacy_hook_policy(doc: &Value) -> bool {
    const LEGACY: &[&str] = &[
        "REMEM_EXECUTOR",
        "REMEM_SUMMARY_EXECUTOR",
        "REMEM_COMPRESS_EXECUTOR",
        "REMEM_DREAM_EXECUTOR",
        "REMEM_MODEL",
        "REMEM_CODEX_MODEL",
        "REMEM_CLAUDE_PATH",
        "REMEM_CODEX_PATH",
        "REMEM_HOOK_ADAPTER",
        "REMEM_CONTEXT_HOST",
        "--gate strict",
        " --color",
    ];
    hook_command_strings(doc).any(|command| LEGACY.iter().any(|needle| command.contains(needle)))
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
    fn probe_hooks_requires_remem_on_each_event() -> anyhow::Result<()> {
        let dir = temp_path("doctor-hooks");
        let hooks_path = dir.join("hooks.json");
        std::fs::write(
            &hooks_path,
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host claude-code" }] }],
    "Stop": [{ "hooks": [{ "command": "other-tool summarize" }] }],
    "PostToolUse": [{ "hooks": [{ "command": "other-tool observe" }] }],
    "UserPromptSubmit": [{ "hooks": [{ "command": "other-tool init" }] }]
  }
}"#,
        )?;
        let mcp_path = dir.join("claude.json");
        std::fs::write(
            &mcp_path,
            r#"{ "mcpServers": { "remem": { "command": "/tmp/remem" } } }"#,
        )?;

        let check = probe_hooks(HostProbe {
            name: "claude",
            hooks_path,
            mcp_paths: vec![mcp_path],
        });

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("1/5 registered"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn probe_hooks_accepts_codex_strategy() -> anyhow::Result<()> {
        let dir = temp_path("doctor-codex-hooks");
        let hooks_path = dir.join("hooks.json");
        std::fs::write(
            &hooks_path,
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host codex-cli" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize --host codex-cli" }] }]
  }
}"#,
        )?;
        let missing_mcp_check = probe_hooks(HostProbe {
            name: "codex",
            hooks_path: hooks_path.clone(),
            mcp_paths: vec![dir.join("missing.toml")],
        });
        assert!(matches!(missing_mcp_check.status, Status::Ok));

        let stale_mcp_path = dir.join("stale.toml");
        std::fs::write(
            &stale_mcp_path,
            "[mcp_servers.remem]\ncommand = \"/stale/remem\"\n",
        )?;
        let stale_mcp_check = probe_hooks(HostProbe {
            name: "codex",
            hooks_path: hooks_path.clone(),
            mcp_paths: vec![stale_mcp_path],
        });
        assert!(matches!(stale_mcp_check.status, Status::Ok));

        let mcp_path = dir.join("config.toml");
        std::fs::write(&mcp_path, "[mcp_servers.remem]\ncommand = \"/tmp/remem\"\n")?;

        let check = probe_hooks(HostProbe {
            name: "codex",
            hooks_path,
            mcp_paths: vec![mcp_path],
        });

        assert!(matches!(check.status, Status::Ok));
        assert!(check.detail.contains("2/2 registered"), "{}", check.detail);
        Ok(())
    }

    #[test]
    fn probe_hooks_rejects_wrong_remem_subcommands_and_hosts() -> anyhow::Result<()> {
        let cases = [
            (
                "doctor-codex-wrong-subcommands",
                r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem status --host codex-cli" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem context --host codex-cli" }] }]
  }
}"#,
            ),
            (
                "doctor-codex-wrong-hosts",
                r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host claude-code" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize --host claude-code" }] }]
  }
}"#,
            ),
        ];

        for (label, content) in cases {
            let dir = temp_path(label);
            let hooks_path = dir.join("hooks.json");
            std::fs::write(&hooks_path, content)?;
            let mcp_path = dir.join("config.toml");
            std::fs::write(&mcp_path, "[mcp_servers.remem]\ncommand = \"/tmp/remem\"\n")?;

            let check = probe_hooks(HostProbe {
                name: "codex",
                hooks_path,
                mcp_paths: vec![mcp_path],
            });

            assert!(
                matches!(check.status, Status::Fail),
                "{label}: {}",
                check.detail
            );
            assert!(
                check.detail.contains("no remem hooks"),
                "{label}: {}",
                check.detail
            );
        }
        Ok(())
    }

    #[test]
    fn probe_hooks_warns_on_codex_posttool_observe() -> anyhow::Result<()> {
        let dir = temp_path("doctor-codex-observe-warning");
        let hooks_path = dir.join("hooks.json");
        std::fs::write(
            &hooks_path,
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host codex-cli" }] }],
    "PostToolUse": [{ "hooks": [{ "command": "/stale/remem observe --host codex-cli" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize --host codex-cli" }] }]
  }
}"#,
        )?;
        let mcp_path = dir.join("config.toml");
        std::fs::write(&mcp_path, "[mcp_servers.remem]\ncommand = \"/tmp/remem\"\n")?;

        let check = probe_hooks(HostProbe {
            name: "codex",
            hooks_path,
            mcp_paths: vec![mcp_path],
        });

        assert!(matches!(check.status, Status::Warn));
        assert!(
            check.detail.contains("PostToolUse observe"),
            "{}",
            check.detail
        );
        Ok(())
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
    fn configured_paths_read_codex_hooks_and_mcp_command() {
        let dir = temp_path("doctor-configured-codex-paths");
        let hooks_path = dir.join("hooks.json");
        let mcp_path = dir.join("config.toml");
        std::fs::write(
            &hooks_path,
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "REMEM_CONTEXT_HOST=codex-cli /hooks/bin/remem context --color" }] }],
    "Stop": [{ "hooks": [{ "command": "REMEM_SUMMARY_EXECUTOR=codex-cli /hooks/bin/remem summarize" }] }]
  }
}"#,
        )
        .unwrap();
        std::fs::write(
            &mcp_path,
            r#"[mcp_servers.remem]
command = "/mcp/bin/remem"
"#,
        )
        .unwrap();

        let paths = configured_remem_paths_for(vec![HostProbe {
            name: "codex",
            hooks_path,
            mcp_paths: vec![mcp_path],
        }]);

        assert!(paths.contains(&PathBuf::from("/hooks/bin/remem")));
        assert!(paths.contains(&PathBuf::from("/mcp/bin/remem")));
    }

    #[test]
    fn configured_paths_read_claude_mcp_command() {
        let dir = temp_path("doctor-configured-claude-paths");
        let mcp_path = dir.join("claude.json");
        std::fs::write(
            &mcp_path,
            r#"{ "mcpServers": { "remem": { "command": "/claude/bin/remem" } } }"#,
        )
        .unwrap();

        let paths = configured_remem_paths_for(vec![HostProbe {
            name: "claude",
            hooks_path: dir.join("settings.json"),
            mcp_paths: vec![mcp_path],
        }]);

        assert_eq!(paths, vec![PathBuf::from("/claude/bin/remem")]);
    }

    #[test]
    fn extract_remem_command_path_ignores_env_assignments() {
        assert_eq!(
            extract_remem_command_path(
                "REMEM_CONTEXT_HOST=codex-cli '/opt/remem/bin/remem' context --color"
            ),
            Some("/opt/remem/bin/remem".to_string())
        );
        assert_eq!(extract_remem_command_path("NOTE=remem echo ok"), None);
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
        let claude_dir = temp_path("doctor-home-claude");
        let codex_dir = temp_path("doctor-home-codex");

        std::fs::write(
            codex_dir.join("hooks.json"),
            r#"{
  "hooks": {
    "SessionStart": [{ "hooks": [{ "command": "/tmp/remem context --host codex-cli" }] }],
    "Stop": [{ "hooks": [{ "command": "/tmp/remem summarize --host codex-cli" }] }]
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
            claude_dir.join("claude.json"),
            r#"{ "mcpServers": { "other": {} } }"#,
        )
        .unwrap();

        let hosts = vec![
            HostProbe {
                name: "claude",
                hooks_path: claude_dir.join("settings.json"),
                mcp_paths: vec![claude_dir.join("claude.json")],
            },
            HostProbe {
                name: "codex",
                hooks_path: codex_dir.join("hooks.json"),
                mcp_paths: vec![codex_dir.join("config.toml")],
            },
        ];

        let hook_checks = check_hooks_for(hosts.clone());
        assert_eq!(hook_checks.len(), 2);
        assert_eq!(hook_checks[0].name, "Hooks (claude)");
        assert!(matches!(hook_checks[0].status, Status::Fail));
        assert_eq!(hook_checks[1].name, "Hooks (codex)");
        assert!(matches!(hook_checks[1].status, Status::Ok));

        let mcp_checks = check_mcp_for(hosts);
        assert_eq!(mcp_checks.len(), 2);
        assert_eq!(mcp_checks[0].name, "MCP (claude)");
        assert!(matches!(mcp_checks[0].status, Status::Fail));
        assert_eq!(mcp_checks[1].name, "MCP (codex)");
        assert!(matches!(mcp_checks[1].status, Status::Ok));
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
