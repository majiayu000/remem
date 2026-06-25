use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use super::audit::{load_memory_audit_rows, DuplicateCluster};
use super::mutate::ObjectMutation;
use super::plan::{apply_memory_cleanup_plan, build_preference_cleanup_plan};
use super::preference_cluster::preference_clusters;

#[derive(Debug, Clone)]
pub struct MergePreferencesRequest<'a> {
    pub project: &'a str,
    pub dry_run: bool,
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MergePreferencesResult {
    pub dry_run: bool,
    pub project: String,
    pub clusters: Vec<DuplicateCluster>,
    pub affected: Vec<ObjectMutation>,
}

pub fn merge_preferences(
    conn: &Connection,
    req: &MergePreferencesRequest<'_>,
) -> Result<MergePreferencesResult> {
    let dry_run = req.dry_run || !req.confirm;
    let memories = load_memory_audit_rows(conn, req.project)?;
    let clusters = preference_clusters(&memories, req.project);
    let mut affected = Vec::new();
    if dry_run || clusters.is_empty() {
        return Ok(MergePreferencesResult {
            dry_run,
            project: req.project.to_string(),
            clusters,
            affected,
        });
    }

    let plan = build_preference_cleanup_plan(conn, req.project)?;
    affected = apply_memory_cleanup_plan(conn, &plan)?.affected;
    Ok(MergePreferencesResult {
        dry_run,
        project: req.project.to_string(),
        clusters,
        affected,
    })
}
