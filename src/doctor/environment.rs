use serde_json::Value;
use std::path::PathBuf;
use toml_edit::DocumentMut;

use super::types::{Check, Status};
use crate::hook_integrity::{
    evaluate_hooks, event_has_remem_subcommand_hook, expected_hook_events,
    expected_hook_executable_from_hooks, extract_remem_command_path, hook_command_strings,
};

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
    let report = expected_executable
        .as_ref()
        .map(|executable| evaluate_hooks(&doc, probe.name, probe.hooks_path.clone(), executable));
    let found = report.as_ref().map(|report| report.registered).unwrap_or(0);
    let deprecated_codex_observe =
        probe.name == "codex" && event_has_remem_subcommand_hook(&doc, "PostToolUse", "observe");
    let legacy_policy = has_legacy_hook_policy(&doc);

    if report.as_ref().is_some_and(|report| report.is_healthy()) {
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
    } else if report.as_ref().is_some_and(|report| {
        found > 0 || (probe.name == "claude" && !report.stale_details.is_empty())
    }) {
        let repair_target = if probe.name == "claude" {
            "claude --repair"
        } else {
            probe.name
        };
        let stale = report
            .as_ref()
            .and_then(|report| report.stale_details.first())
            .map(|detail| format!("; {detail}"))
            .unwrap_or_default();
        Check::new(
            name,
            Status::Warn,
            format!(
                "{}/{} registered{} (run `remem install --target {}` to fix)",
                found,
                events.len(),
                stale,
                repair_target
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
    configured_mcp_paths(probe)
        .into_iter()
        .next()
        .or_else(|| expected_hook_executable_from_hooks(doc, probe.name).map(PathBuf::from))
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
        .filter_map(|path| {
            if crate::hook_cli::is_full_remem_binary(&path) {
                Some(path)
            } else if crate::hook_cli::is_hook_binary(&path) {
                crate::hook_cli::sibling_full_binary_path(&path)
            } else {
                None
            }
        })
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
mod tests;
