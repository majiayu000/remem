use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::{single_line, LoadedPack, PackImportCategory, PackImportPlan};
use crate::cli::actions::pack_export::PackMemory;

const PACK_SOURCE_KIND: &str = "pack";
const PACK_TRUST_CLASS: crate::memory::poisoning::SourceTrustClass =
    crate::memory::poisoning::SourceTrustClass::Pack;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PackImportApplyReport {
    pub(super) plan: PackImportPlan,
    pub(super) applied: PackImportApplyStats,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct PackImportApplyStats {
    pub(super) added_memories: usize,
    pub(super) pending_review_candidates: usize,
    pub(super) quarantined_candidates: usize,
}

pub(super) fn apply_loaded_pack(
    conn: &mut Connection,
    target_project: &str,
    loaded: LoadedPack,
) -> Result<PackImportApplyReport> {
    let tx = conn.transaction()?;
    let project_id = ensure_project_row(&tx, target_project)?;
    let plan = super::plan_loaded_pack(Some(&tx), target_project, loaded)?;
    let mut applied = PackImportApplyStats::default();

    for entry in &plan.entries {
        match entry.category {
            PackImportCategory::Add => {
                insert_pack_memory(&tx, target_project, &entry.memory, &plan.content_digest)?;
                applied.added_memories += 1;
            }
            PackImportCategory::Conflict => {
                insert_pack_candidate(
                    &tx,
                    project_id,
                    target_project,
                    &entry.memory,
                    &plan.content_digest,
                    "pending_review",
                    "pack_import_conflict",
                    None,
                )?;
                applied.pending_review_candidates += 1;
            }
            PackImportCategory::Quarantine => {
                let matched = crate::memory::poisoning::scan_instruction_pattern(&format!(
                    "{}\n{}",
                    entry.memory.title, entry.memory.content
                ))
                .context("quarantine plan row lost instruction-pattern match before apply")?;
                insert_pack_candidate(
                    &tx,
                    project_id,
                    target_project,
                    &entry.memory,
                    &plan.content_digest,
                    "quarantined",
                    "quarantined_instruction_pattern",
                    Some(matched),
                )?;
                applied.quarantined_candidates += 1;
            }
            PackImportCategory::Dedup | PackImportCategory::Skip => {}
        }
    }

    tx.commit()?;
    Ok(PackImportApplyReport { plan, applied })
}

pub(super) fn render_import_apply_report(pack: &Path, report: &PackImportApplyReport) -> String {
    let mut output = format!(
        "Pack import applied for {}: added={} pending_review={} quarantined={} (planned add={} dedup={} skip={} conflict={} quarantine={}, digest {}).\n",
        pack.display(),
        report.applied.added_memories,
        report.applied.pending_review_candidates,
        report.applied.quarantined_candidates,
        report.plan.stats.add,
        report.plan.stats.dedup,
        report.plan.stats.skip,
        report.plan.stats.conflict,
        report.plan.stats.quarantine,
        report.plan.content_digest
    );
    for entry in &report.plan.entries {
        output.push_str(&format!(
            "- {} state_key={} hash={} title=\"{}\" reason=\"{}\"\n",
            entry.category.as_str(),
            entry.state_key.as_deref().unwrap_or("-"),
            entry.content_hash,
            single_line(&entry.title),
            single_line(&entry.reason)
        ));
    }
    output
}

fn ensure_project_row(conn: &Connection, target_project: &str) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch)
         VALUES (?1, NULL, NULL, ?2, ?2)
         ON CONFLICT(root_path) DO UPDATE SET updated_at_epoch = excluded.updated_at_epoch",
        params![target_project, now],
    )?;
    let workspace_id: i64 = conn.query_row(
        "SELECT id FROM workspaces WHERE root_path = ?1",
        params![target_project],
        |row| row.get(0),
    )?;
    let project_key = target_project
        .rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(target_project);
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(workspace_id, project_path) DO UPDATE SET
             project_key = excluded.project_key,
             updated_at_epoch = excluded.updated_at_epoch",
        params![workspace_id, target_project, project_key, now],
    )?;
    conn.query_row(
        "SELECT id FROM projects WHERE workspace_id = ?1 AND project_path = ?2",
        params![workspace_id, target_project],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn insert_pack_memory(
    conn: &Connection,
    target_project: &str,
    memory: &PackMemory,
    content_digest: &str,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let reference_time = memory.valid_from_epoch.unwrap_or(memory.created_at_epoch);
    let search_context = crate::memory::search_context::build_search_context(
        &memory.memory_type,
        memory.state_key.as_deref(),
        &memory.content,
        None,
    );
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          evidence_event_ids, source_candidate_id, confidence,
          source_project, target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, routing_reason, context_class, expires_at_epoch,
          valid_from_epoch, source_trust_class)
         VALUES (NULL, ?1, ?2, ?3, ?4, ?5, NULL, ?6,
                 ?7, ?8, ?9, 'active', NULL, 'project',
                 '[]', NULL, ?10,
                 ?1, ?1, 'repo', ?1, ?11,
                 1.0, ?12, 'startup_core', ?13,
                 ?14, ?15)",
        params![
            target_project,
            memory.state_key.as_deref(),
            memory.title.as_str(),
            memory.content.as_str(),
            memory.memory_type.as_str(),
            search_context,
            memory.created_at_epoch,
            now,
            reference_time,
            memory.confidence,
            pack_topic_domain(content_digest),
            "pack import",
            memory.expires_at_epoch,
            memory.valid_from_epoch,
            PACK_TRUST_CLASS.as_str(),
        ],
    )?;
    let memory_id = conn.last_insert_rowid();
    if let Some(state_key) = memory.state_key.as_deref() {
        crate::memory::state_key::attach_current_memory(
            conn,
            memory_id,
            "repo",
            target_project,
            &memory.memory_type,
            &crate::memory::state_key::StateKeyDecision {
                state_key: state_key.to_string(),
                confidence: memory.state_key_confidence.unwrap_or(1.0),
                reason: memory
                    .state_key_reason
                    .clone()
                    .unwrap_or_else(|| "pack_import".to_string()),
            },
            now,
        )?;
    }
    refresh_imported_memory_indexes(conn, memory_id, &memory.title, &memory.content)?;
    Ok(memory_id)
}

fn insert_pack_candidate(
    conn: &Connection,
    project_id: i64,
    target_project: &str,
    memory: &PackMemory,
    content_digest: &str,
    review_status: &str,
    block_reason: &str,
    quarantine_match: Option<crate::memory::poisoning::InstructionPatternMatch>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let topic_key = memory
        .state_key
        .as_deref()
        .unwrap_or(memory.content_hash.as_str());
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
          auto_promote_block_reason, source_project, target_project, owner_scope, owner_key,
          topic_domain, routing_confidence, routing_reason, context_class, expires_at_epoch,
          valid_from_epoch, state_key, state_key_confidence, state_key_reason,
          source_kind, source_trust_class, quarantine_pattern_id, quarantine_pattern_version)
         VALUES (?1, 'project', ?2, ?3, ?4, '[]',
                 ?5, ?6, ?7, ?8, ?8,
                 ?9, ?10, ?10, 'repo', ?10,
                 ?11, 1.0, 'pack import', 'startup_core', ?12,
                 ?13, ?14, ?15, ?16,
                 ?17, ?18, ?19, ?20)",
        params![
            project_id,
            memory.memory_type.as_str(),
            topic_key,
            memory.content.as_str(),
            memory.confidence.unwrap_or(0.5),
            if review_status == "quarantined" {
                "high"
            } else {
                "medium"
            },
            review_status,
            now,
            block_reason,
            target_project,
            pack_topic_domain(content_digest),
            memory.expires_at_epoch,
            memory.valid_from_epoch,
            memory.state_key.as_deref(),
            memory.state_key_confidence,
            memory.state_key_reason.as_deref(),
            PACK_SOURCE_KIND,
            PACK_TRUST_CLASS.as_str(),
            quarantine_match.map(|matched| matched.pattern_id),
            quarantine_match.map(|matched| matched.pattern_set_version),
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn pack_topic_domain(digest: &str) -> String {
    let prefix = digest.chars().take(12).collect::<String>();
    format!("pack:{prefix}")
}

fn refresh_imported_memory_indexes(
    conn: &Connection,
    memory_id: i64,
    title: &str,
    content: &str,
) -> Result<()> {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    crate::retrieval::entity::refresh_memory_entities(conn, memory_id, &entities)?;
    crate::retrieval::vector::upsert_memory_embedding_for_row(conn, memory_id)
}
