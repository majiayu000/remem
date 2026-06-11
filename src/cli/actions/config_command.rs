use anyhow::Result;
use serde_json::json;

use crate::cli::types::ConfigAction;
use crate::runtime_config::LegacyClaudeGateMigration;

pub(in crate::cli) fn run_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Path => {
            println!("{}", crate::runtime_config::config_path().display());
        }
        ConfigAction::Show => {
            print!("{}", crate::runtime_config::show_config_text()?);
        }
        ConfigAction::Init => {
            let path = crate::runtime_config::init_config()?;
            println!("config -> {}", path.display());
        }
        ConfigAction::Set { key, value } => {
            let path = crate::runtime_config::set_config_value(&key, &value)?;
            println!("config -> {}", path.display());
        }
        ConfigAction::MigrateClaudeGate { dry_run, json } => {
            let migration = crate::runtime_config::migrate_legacy_claude_context_gate(dry_run)?;
            if json {
                print_migration_json(&migration);
            } else {
                print_migration_human(&migration);
            }
        }
    }
    Ok(())
}

fn print_migration_human(migration: &LegacyClaudeGateMigration) {
    println!("Config: {}", migration.config_path.display());
    println!("Host: {}", migration.host);
    if migration.dry_run {
        println!("Mode: dry-run (no files written)");
    }
    if migration.changed {
        println!(
            "context_gate: {} -> {}",
            quote_or_dash(migration.old_gate.as_deref()),
            quote_or_dash(migration.new_gate.as_deref())
        );
    } else {
        println!(
            "context_gate: {} (no migration needed)",
            quote_or_dash(migration.new_gate.as_deref())
        );
    }
}

fn print_migration_json(migration: &LegacyClaudeGateMigration) {
    println!(
        "{}",
        json!({
            "config_path": migration.config_path,
            "host": migration.host,
            "old_gate": migration.old_gate,
            "new_gate": migration.new_gate,
            "changed": migration.changed,
            "dry_run": migration.dry_run
        })
    );
}

fn quote_or_dash(value: Option<&str>) -> String {
    value
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "-".to_string())
}
