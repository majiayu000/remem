//! Transactional apply (GH-852 B-009, B-010): all planned records are written
//! as review candidates in a single transaction, or none are. The source tree
//! is never written. Records never reach active memories from this path.

use anyhow::Result;
use rusqlite::Connection;

use crate::memory_candidate::route::{
    insert_external_candidate, ExternalCandidateInsert, ExternalCandidateOutcome,
};

use super::plan::{topic_key_for, DestinationRoute, ImportPlan, SOURCE_KIND};

const TOOL_OWNED_PROJECT: &str = "tool:codex-cli";

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ApplySummary {
    pub pending_review: usize,
    pub quarantined: usize,
    pub dedup: usize,
}

pub(super) fn apply_plan(mut conn: Connection, plan: &ImportPlan) -> Result<ApplySummary> {
    let tx = conn.transaction()?;
    let mut summary = ApplySummary::default();
    for entry in &plan.entries {
        let (project, owner_scope, owner_key, target_project, context_class, routing_reason) =
            match &entry.route {
                DestinationRoute::Project(project) => (
                    project.clone(),
                    "repo",
                    project.as_str(),
                    Some(project.as_str()),
                    "startup_core",
                    "verified codex workspace evidence (record cwd)",
                ),
                DestinationRoute::ToolOwned => (
                    TOOL_OWNED_PROJECT.to_string(),
                    "tool",
                    "codex-cli",
                    None,
                    "search_only",
                    "unverifiable codex workspace evidence; tool-owned review",
                ),
            };
        let project_id = crate::db::capture::ensure_project_row(&tx, &project)?;
        let topic_key = topic_key_for(&entry.identity);
        let outcome = insert_external_candidate(
            &tx,
            &ExternalCandidateInsert {
                project_id,
                source_project: &project,
                scope: "project",
                memory_type: "discovery",
                topic_key: &topic_key,
                text: &entry.record.body,
                confidence: 0.5,
                risk_class: "high",
                source_kind: SOURCE_KIND,
                semantic_discriminator_sha256: None,
                owner_scope,
                owner_key,
                target_project,
                context_class,
                routing_reason,
                quarantine_match: None,
            },
        )?;
        match outcome {
            ExternalCandidateOutcome::Inserted {
                quarantined: true, ..
            } => summary.quarantined += 1,
            ExternalCandidateOutcome::Inserted {
                quarantined: false, ..
            } => summary.pending_review += 1,
            ExternalCandidateOutcome::Duplicate { .. } => summary.dedup += 1,
        }
    }
    tx.commit()?;
    Ok(summary)
}
