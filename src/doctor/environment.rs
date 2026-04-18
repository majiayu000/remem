use std::path::PathBuf;

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
struct HostProbe {
    name: &'static str,
    hooks_path: PathBuf,
    mcp_path: PathBuf,
    /// Needle in the MCP config file that indicates remem is registered.
    /// JSON hosts use `mcpServers`, TOML hosts use `mcp_servers`.
    mcp_needle: &'static str,
}

fn known_hosts() -> Vec<HostProbe> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    vec![
        HostProbe {
            name: "claude",
            hooks_path: home.join(".claude").join("settings.json"),
            mcp_path: home.join(".claude.json"),
            mcp_needle: "mcpServers",
        },
        HostProbe {
            name: "codex",
            hooks_path: home.join(".codex").join("hooks.json"),
            mcp_path: home.join(".codex").join("config.toml"),
            mcp_needle: "mcp_servers",
        },
    ]
}

/// True if the host's config directory exists — i.e. the tool is installed
/// on this machine and worth probing.
fn host_present(probe: &HostProbe) -> bool {
    probe.hooks_path.parent().is_some_and(|p| p.exists())
        || probe.hooks_path.exists()
        || probe.mcp_path.exists()
}

/// Produce one Check per detected host's hooks file. Hosts whose config
/// directory doesn't exist are silently skipped — they aren't installed, so
/// there's nothing to validate.
pub(super) fn check_hooks() -> Vec<Check> {
    let mut checks = Vec::new();
    for probe in known_hosts().iter().filter(|h| host_present(h)) {
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
    for probe in known_hosts().iter().filter(|h| host_present(h)) {
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

fn probe_hooks(probe: &HostProbe) -> Check {
    let name: &'static str = Box::leak(format!("Hooks ({})", probe.name).into_boxed_str());

    if !probe.hooks_path.exists() {
        return Check {
            name,
            status: Status::Fail,
            detail: format!("{} not found (run `remem install`)", probe.hooks_path.display()),
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

    let events = ["PostToolUse", "Stop", "SessionStart", "UserPromptSubmit"];
    let found = events
        .iter()
        .filter(|event| content.contains(*event) && content.contains("remem"))
        .count();

    if found == events.len() {
        Check {
            name,
            status: Status::Ok,
            detail: format!("{}/{} registered in {}", found, events.len(), probe.hooks_path.display()),
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
            detail: format!("no remem hooks (run `remem install --target {}`)", probe.name),
        }
    }
}

fn probe_mcp(probe: &HostProbe) -> Check {
    let name: &'static str = Box::leak(format!("MCP ({})", probe.name).into_boxed_str());

    if !probe.mcp_path.exists() {
        return Check {
            name,
            status: Status::Fail,
            detail: format!(
                "{} not found (run `remem install --target {}`)",
                probe.mcp_path.display(),
                probe.name
            ),
        };
    }

    let content = match std::fs::read_to_string(&probe.mcp_path) {
        Ok(c) => c,
        Err(err) => {
            return Check {
                name,
                status: Status::Fail,
                detail: format!("cannot read {}: {}", probe.mcp_path.display(), err),
            };
        }
    };

    // Substring check is defensive (works for both JSON and TOML without
    // parsing either). `<needle>` + `remem` both present ≈ registered.
    if content.contains(probe.mcp_needle) && content.contains("remem") {
        Check {
            name,
            status: Status::Ok,
            detail: format!("registered in {}", probe.mcp_path.display()),
        }
    } else {
        Check {
            name,
            status: Status::Fail,
            detail: format!(
                "not registered (run `remem install --target {}`)",
                probe.name
            ),
        }
    }
}
