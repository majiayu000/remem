//! Dry-run-first application of a digest-bound project alias inventory report.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::Parser;
use remem::project_alias::{
    apply_project_alias_plan, preview_project_alias_plan, ProjectAliasApplyRequest,
    ProjectAliasPlanEntry,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Parser)]
#[command(about = "preview or apply a project alias inventory report")]
struct Args {
    /// JSON emitted by the project_alias_inventory example.
    plan: PathBuf,
    /// Actor stored in append-only alias audit events.
    #[arg(long)]
    actor: String,
    /// Human-readable reason stored in append-only alias audit events.
    #[arg(long)]
    reason: String,
    /// Apply the plan. Omit for a read-only preview.
    #[arg(long)]
    apply: bool,
}

#[derive(Debug, Deserialize)]
struct InventoryReport {
    schema_version: u32,
    inventory_sha256: String,
    proposed_aliases: Vec<ProjectAliasPlanEntry>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let raw = std::fs::read_to_string(&args.plan)
        .with_context(|| format!("read project alias plan {}", args.plan.display()))?;
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parse project alias plan {}", args.plan.display()))?;
    verify_inventory_digest(&value)?;
    let report: InventoryReport = serde_json::from_value(value)?;
    if report.schema_version != 1 {
        bail!(
            "unsupported project alias inventory schema version {}",
            report.schema_version
        );
    }
    let request = ProjectAliasApplyRequest {
        source_inventory_sha256: &report.inventory_sha256,
        actor: &args.actor,
        reason: &args.reason,
        now_epoch: Utc::now().timestamp(),
        entries: &report.proposed_aliases,
    };
    let result = if args.apply {
        let conn = remem::db::open_db_no_migrate()
            .context("open current-schema database for project alias apply")?;
        apply_project_alias_plan(&conn, &request)?
    } else {
        let conn = remem::db::open_db_read_only_current()
            .context("open current-schema database for project alias preview")?;
        preview_project_alias_plan(&conn, &request)?
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1,
            "mode": if args.apply { "applied" } else { "dry_run" },
            "source_inventory_sha256": report.inventory_sha256,
            "inserted": result.inserted,
            "unchanged": result.unchanged,
            "aliases": result.aliases
        }))?
    );
    Ok(())
}

fn verify_inventory_digest(value: &Value) -> Result<()> {
    let mut canonical = value.clone();
    let object = canonical
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("project alias inventory must be an object"))?;
    let declared = object
        .get("inventory_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("project alias inventory is missing inventory_sha256"))?
        .to_string();
    object.insert("inventory_sha256".to_string(), Value::String(String::new()));
    let actual = format!("{:x}", Sha256::digest(serde_json::to_vec(&canonical)?));
    if actual != declared {
        bail!(
            "project alias inventory digest mismatch: declared {}, actual {}",
            declared,
            actual
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_digest_detects_drift() -> Result<()> {
        let mut value = serde_json::json!({
            "schema_version": 1,
            "inventory_sha256": "",
            "proposed_aliases": []
        });
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&value)?));
        value["inventory_sha256"] = Value::String(digest);
        verify_inventory_digest(&value)?;
        value["schema_version"] = serde_json::json!(2);
        assert!(verify_inventory_digest(&value).is_err());
        Ok(())
    }
}
