use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::cli::types::ImportAction;

pub(in crate::cli) fn run_import(action: ImportAction) -> Result<()> {
    match action {
        ImportAction::Legacy {
            source,
            best_effort,
        } => run_import_legacy(source, best_effort),
    }
}

fn run_import_legacy(source: PathBuf, best_effort: bool) -> Result<()> {
    if !best_effort {
        anyhow::bail!(
            "import legacy currently only supports --best-effort mode (per SPEC §17 D5)."
        );
    }
    let v2_conn = crate::v2_db::open_v2_db().context("open v2 database for import target")?;
    let stats = crate::v2_import::import_legacy_memories(&source, &v2_conn)
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
