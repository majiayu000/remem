#[cfg(unix)]
use std::process::Command;

use super::types::{Check, Status};

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpProcess {
    pid: u32,
    command: String,
    args: String,
}

pub(super) fn check_mcp_processes() -> Check {
    #[cfg(not(unix))]
    {
        return Check::new(
            "MCP processes",
            Status::Ok,
            "MCP process scan skipped on this platform",
        );
    }

    #[cfg(unix)]
    match active_mcp_processes_from_system() {
        Ok(processes) if processes.is_empty() => Check::new(
            "MCP processes",
            Status::Ok,
            "no active remem mcp processes detected",
        ),
        Ok(processes) => Check::new(
            "MCP processes",
            Status::Warn,
            format!(
                "{} active remem mcp process(es) detected; after binary or schema upgrades, restart Codex/Claude sessions so MCP reconnects to the upgraded remem binary",
                processes.len()
            ),
        ),
        Err(err) => Check::new(
            "MCP processes",
            Status::Warn,
            format!("process scan unavailable: {err}"),
        ),
    }
}

#[cfg(unix)]
fn active_mcp_processes_from_system() -> anyhow::Result<Vec<McpProcess>> {
    let output = Command::new("ps")
        .args(["-axo", "pid=,comm=,args="])
        .output()?;
    if !output.status.success() {
        anyhow::bail!("ps exited with {}", output.status);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_mcp_processes(&stdout, std::process::id()))
}

fn parse_mcp_processes(ps_output: &str, current_pid: u32) -> Vec<McpProcess> {
    ps_output
        .lines()
        .filter_map(parse_ps_line)
        .filter(|process| process.pid != current_pid)
        .filter(|process| is_remem_mcp_process(&process.command, &process.args))
        .collect()
}

fn parse_ps_line(line: &str) -> Option<McpProcess> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.splitn(3, char::is_whitespace);
    let pid = parts.next()?.parse::<u32>().ok()?;
    let command = parts.next()?.trim().to_string();
    let args = parts.next().unwrap_or_default().trim().to_string();
    if command.is_empty() {
        return None;
    }
    if args.is_empty() {
        return None;
    }
    Some(McpProcess { pid, command, args })
}

fn is_remem_mcp_process(command: &str, args: &str) -> bool {
    let command = command.trim_matches('"').trim_matches('\'');
    let has_remem_binary = command == "remem" || command == "remem.exe";
    let tokens = args.split_whitespace().collect::<Vec<_>>();
    has_remem_binary && tokens.contains(&"mcp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_detects_remem_mcp_and_ignores_current_process() {
        let output = "\
            10 remem /opt/homebrew/bin/remem mcp\n\
            11 grep /usr/bin/grep remem mcp\n\
            12 remem /Users/me/.local/bin/remem context --host codex-cli\n\
            13 remem /Users/me/.local/bin/remem mcp\n\
            14 remem /Users/Alice Smith/.local/bin/remem mcp\n";

        let processes = parse_mcp_processes(output, 13);

        assert_eq!(
            processes,
            vec![
                McpProcess {
                    pid: 10,
                    command: "remem".to_string(),
                    args: "/opt/homebrew/bin/remem mcp".to_string()
                },
                McpProcess {
                    pid: 14,
                    command: "remem".to_string(),
                    args: "/Users/Alice Smith/.local/bin/remem mcp".to_string()
                }
            ]
        );
    }

    #[test]
    fn check_warns_when_processes_are_active() {
        let processes = vec![McpProcess {
            pid: 10,
            command: "remem".to_string(),
            args: "/opt/homebrew/bin/remem mcp".to_string(),
        }];
        let check = if processes.is_empty() {
            Check::new(
                "MCP processes",
                Status::Ok,
                "no active remem mcp processes detected",
            )
        } else {
            Check::new(
                "MCP processes",
                Status::Warn,
                format!(
                    "{} active remem mcp process(es) detected; after binary or schema upgrades, restart Codex/Claude sessions so MCP reconnects to the upgraded remem binary",
                    processes.len()
                ),
            )
        };

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("restart Codex/Claude sessions"));
    }

    #[cfg(not(unix))]
    #[test]
    fn check_skips_process_scan_on_non_unix_platforms() {
        let check = check_mcp_processes();

        assert!(matches!(check.status, Status::Ok));
        assert!(check.detail.contains("skipped"));
    }
}
