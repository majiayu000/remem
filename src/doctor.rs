use anyhow::Result;
use std::path::PathBuf;

use crate::db;

struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

enum Status {
    Ok,
    Warn,
    Fail,
}

impl Check {
    fn icon(&self) -> &'static str {
        match self.status {
            Status::Ok => "ok",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
        }
    }
}

pub fn run_doctor() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!("remem v{} — system check", version);
    println!();

    let checks = vec![
        check_binary(),
        check_schema_migration(),
        check_database(),
        check_hooks(),
        check_mcp(),
        check_pending_queue(),
        check_disk_space(),
    ];

    let mut warns = 0;
    let mut fails = 0;

    for check in &checks {
        let marker = check.icon();
        println!("  [{}] {}: {}", marker, check.name, check.detail);
        match check.status {
            Status::Warn => warns += 1,
            Status::Fail => fails += 1,
            Status::Ok => {}
        }
    }

    println!();
    if fails > 0 {
        println!(
            "{} check(s) failed, {} warning(s). Run `remem install` to fix hook/MCP issues.",
            fails, warns
        );
    } else if warns > 0 {
        println!("All checks passed with {} warning(s).", warns);
    } else {
        println!("All checks passed.");
    }

    Ok(())
}

fn check_binary() -> Check {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    Check {
        name: "Binary",
        status: Status::Ok,
        detail: exe,
    }
}

fn check_database() -> Check {
    let db_path = db::db_path();
    if !db_path.exists() {
        return Check {
            name: "Database",
            status: Status::Fail,
            detail: format!("{} (not found)", db_path.display()),
        };
    }

    let size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    match db::open_db() {
        Ok(conn) => {
            let memory_count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE status = 'active'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Check {
                name: "Database",
                status: Status::Ok,
                detail: format!(
                    "{} ({:.1} MB, {} memories)",
                    db_path.display(),
                    size as f64 / 1_048_576.0,
                    memory_count
                ),
            }
        }
        Err(e) => Check {
            name: "Database",
            status: Status::Fail,
            detail: format!("{} (open error: {})", db_path.display(), e),
        },
    }
}

fn check_hooks() -> Check {
    let settings_path = dirs::home_dir()
        .map(|h| h.join(".claude").join("settings.json"))
        .unwrap_or_else(|| PathBuf::from("~/.claude/settings.json"));

    if !settings_path.exists() {
        return Check {
            name: "Hooks",
            status: Status::Fail,
            detail: format!("{} not found", settings_path.display()),
        };
    }

    let content = match std::fs::read_to_string(&settings_path) {
        Ok(c) => c,
        Err(e) => {
            return Check {
                name: "Hooks",
                status: Status::Fail,
                detail: format!("cannot read {}: {}", settings_path.display(), e),
            }
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

fn check_mcp() -> Check {
    // Check both possible MCP config locations
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

fn check_pending_queue() -> Check {
    let conn = match db::open_db() {
        Ok(c) => c,
        Err(_) => {
            return Check {
                name: "Pending queue",
                status: Status::Warn,
                detail: "cannot open database".to_string(),
            }
        }
    };

    let pending: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE status = 'pending'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let failed_pending: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE status = 'failed'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    let stuck_jobs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM jobs WHERE state = 'running' \
             AND lease_expires_epoch < strftime('%s', 'now')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    if stuck_jobs > 0 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!(
                "{} pending, {} failed, {} stuck jobs (will auto-recover)",
                pending, failed_pending, stuck_jobs
            ),
        }
    } else if failed_pending > 0 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!(
                "{} pending, {} failed (inspect parsing/AI failures)",
                pending, failed_pending
            ),
        }
    } else if pending > 100 {
        Check {
            name: "Pending queue",
            status: Status::Warn,
            detail: format!(
                "{} pending, {} failed (backlog building up)",
                pending, failed_pending
            ),
        }
    } else {
        Check {
            name: "Pending queue",
            status: Status::Ok,
            detail: format!("{} pending, {} failed", pending, failed_pending),
        }
    }
}

/// Dry-run migration check: copy real table schemas into an in-memory DB,
/// then execute pending migrations. Catches issues before they break hooks.
fn check_schema_migration() -> Check {
    let db_path = db::db_path();
    if !db_path.exists() {
        return Check {
            name: "Schema",
            status: Status::Ok,
            detail: "no database yet (will create on first use)".into(),
        };
    }

    let real_conn = match rusqlite::Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            return Check {
                name: "Schema",
                status: Status::Fail,
                detail: format!("cannot open DB: {}", e),
            }
        }
    };
    if let Err(e) = real_conn.execute_batch("PRAGMA busy_timeout=5000;") {
        return Check {
            name: "Schema",
            status: Status::Fail,
            detail: format!("cannot set busy_timeout: {}", e),
        };
    }

    match crate::migrate::dry_run_pending(&real_conn) {
        Ok(result) => {
            if result.pending_count == 0 {
                Check {
                    name: "Schema",
                    status: Status::Ok,
                    detail: format!("v{} (up to date)", result.current_version),
                }
            } else if let Some(err) = result.error {
                Check {
                    name: "Schema",
                    status: Status::Fail,
                    detail: format!(
                        "{} pending migration(s) will FAIL: {}",
                        result.pending_count, err
                    ),
                }
            } else {
                Check {
                    name: "Schema",
                    status: Status::Ok,
                    detail: format!(
                        "v{} ({} pending migration(s), dry-run passed)",
                        result.current_version, result.pending_count
                    ),
                }
            }
        }
        Err(e) => Check {
            name: "Schema",
            status: Status::Fail,
            detail: format!("dry-run error: {}", e),
        },
    }
}

fn check_disk_space() -> Check {
    let db_path = db::db_path();
    let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
    let log_path = db_path.parent().map(|p| p.join("remem.log"));
    let log_size = log_path
        .and_then(|p| std::fs::metadata(&p).ok())
        .map(|m| m.len())
        .unwrap_or(0);

    let total_mb = (db_size + log_size) as f64 / 1_048_576.0;

    if total_mb > 500.0 {
        Check {
            name: "Disk usage",
            status: Status::Warn,
            detail: format!(
                "{:.1} MB total (DB: {:.1} MB, logs: {:.1} MB) — consider `remem cleanup`",
                total_mb,
                db_size as f64 / 1_048_576.0,
                log_size as f64 / 1_048_576.0
            ),
        }
    } else {
        Check {
            name: "Disk usage",
            status: Status::Ok,
            detail: format!(
                "{:.1} MB total (DB: {:.1} MB, logs: {:.1} MB)",
                total_mb,
                db_size as f64 / 1_048_576.0,
                log_size as f64 / 1_048_576.0
            ),
        }
    }
}
