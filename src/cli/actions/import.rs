use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cli::types::ImportAction;

pub(in crate::cli) fn run_import(action: ImportAction) -> Result<()> {
    match action {
        ImportAction::Backup {
            source,
            best_effort,
        } => run_import_backup(source, best_effort),
    }
}

fn run_import_backup(source: PathBuf, best_effort: bool) -> Result<()> {
    if !best_effort {
        anyhow::bail!("backup import currently only supports --best-effort mode.");
    }
    let schema_conn =
        crate::db::schema::open().context("open schema database for import target")?;
    let stats = crate::db::schema::import::import_memories(&source, &schema_conn)
        .with_context(|| format!("import from {}", source.display()))?;
    println!(
        "Imported {} memories ({} skipped). Created {} workspaces, {} projects.",
        stats.memories_imported,
        stats.memories_skipped,
        stats.workspaces_created,
        stats.projects_created,
    );
    Ok(())
}
