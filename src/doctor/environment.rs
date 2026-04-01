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

pub(super) fn check_hooks() -> Check {
    let settings_path = dirs::home_dir()
        .map(|home| home.join(".claude").join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("~/.claude/settings.json"));

    if !settings_path.exists() {
        return Check {
            name: "Hooks",
            status: Status::Fail,
            detail: format!("{} not found", settings_path.display()),
        };
    }

    let content = match std::fs::read_to_string(&settings_path) {
        Ok(content) => content,
        Err(err) => {
            return Check {
                name: "Hooks",
                status: Status::Fail,
                detail: format!("cannot read {}: {}", settings_path.display(), err),
            };
        }
    };

    let hooks = ["PostToolUse", "Stop", "SessionStart", "UserPromptSubmit"];
    let mut found = 0;
    for hook in &hooks {
        if content.contains(hook) && content.contains("remem") {
            found += 1;
        }
    }

    if found == hooks.len() {
        Check {
            name: "Hooks",
            status: Status::Ok,
            detail: format!("{}/{} registered in settings.json", found, hooks.len()),
        }
    } else if found > 0 {
        Check {
            name: "Hooks",
            status: Status::Warn,
            detail: format!(
                "{}/{} registered (run `remem install` to fix)",
                found,
                hooks.len()
            ),
        }
    } else {
        Check {
            name: "Hooks",
            status: Status::Fail,
            detail: "no remem hooks found (run `remem install`)".to_string(),
        }
    }
}

pub(super) fn check_mcp() -> Check {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mcp_paths = [
        home.join(".claude.json"),
        home.join(".claude").join("claude_desktop_config.json"),
    ];

    for path in &mcp_paths {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                if content.contains("remem") && content.contains("mcp") {
                    return Check {
                        name: "MCP server",
                        status: Status::Ok,
                        detail: format!("registered in {}", path.display()),
                    };
                }
            }
        }
    }

    Check {
        name: "MCP server",
        status: Status::Fail,
        detail: "not registered (run `remem install`)".to_string(),
    }
}
