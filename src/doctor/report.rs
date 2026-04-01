use anyhow::Result;

use super::database::{check_database, check_disk_space, check_pending_queue};
use super::environment::{check_binary, check_hooks, check_mcp};
use super::schema::check_schema_migration;
use super::types::Status;

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
        println!("  [{}] {}: {}", check.icon(), check.name, check.detail);
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
