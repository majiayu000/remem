//! Pre-v076 Dream-merge stock backfill (GH-990).
//!
//! v076 closed the forward half of the Dream poisoning boundary: new Dream
//! model output is scanned before persistence and quarantined on a match.
//! Memories merged by Dream before v076 kept the default `local_tool_output`
//! trust class and stayed active. This module re-scans that stock with the
//! same generated-surface scanner and the same calling convention as the
//! forward path (`src/dream/poisoning.rs`):
//!
//! - **hit** → the memory is retired (`status='archived'`) and bound to a
//!   quarantine artifact + review candidate through the existing ledger, so
//!   review approval restores it in place and rejection leaves it retired
//!   with a full audit trail;
//! - **no hit** → only `source_trust_class` is backfilled to
//!   `external_content`, matching what the forward path stamps on new rows.
//!
//! Nothing here runs inside a migration. The ledger is append-only and its
//! rows can neither be updated nor deleted, so execution is an explicit CLI
//! decision (`remem dream-backfill`); planning is always side-effect free.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::memory::poisoning::{scan_generated_surfaces, SurfacePatternMatch};
use crate::memory_candidate::route::{
    insert_external_candidate, ExternalCandidateInsert, ExternalCandidateOutcome,
};

const STOCK_SESSION_ID: &str = "dream";
const STOCK_TRUST_CLASS: &str = "local_tool_output";
const DREAM_TRUST_CLASS: &str = "external_content";
const SOURCE_KIND: &str = "dream_model_output";
const BACKFILL_ACTOR: &str = "dream-backfill";
const BACKFILL_SOURCE: &str = "dream_backfill";
const BACKFILL_PLANNER_VERSION: &str = "gh990-v1";

/// One stock row: a Dream-merged memory written before the v076 boundary.
#[derive(Debug, Clone)]
struct StockMemory {
    id: i64,
    project: String,
    topic_key: Option<String>,
    memory_type: String,
    title: String,
    content: String,
    version: i64,
    updated_at_epoch: i64,
}

impl StockMemory {
    /// Artifact-safe topic key: the merge CHECK requires a non-empty value,
    /// so a NULL topic falls back to a deterministic per-memory key, mirroring
    /// the forward path's `dream-quarantine-{signature}` fallback.
    fn effective_topic_key(&self) -> String {
        self.topic_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("dream-backfill-{}", self.id))
    }

    /// The artifact CHECK requires non-empty generated title/content; rows
    /// that cannot satisfy it are reported as skipped instead of quarantined.
    fn quarantinable(&self) -> bool {
        !self.title.trim().is_empty() && !self.content.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BackfillHit {
    pub memory_id: i64,
    pub project: String,
    pub title: String,
    pub snapshot_sha256: String,
    pub matched_field: String,
    pub pattern_id: String,
    pub pattern_version: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackfillSkip {
    pub memory_id: i64,
    pub project: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackfillNoHit {
    pub memory_id: i64,
    pub project: String,
    pub snapshot_sha256: String,
}

#[derive(Debug, Default, Serialize)]
pub struct BackfillPlan {
    pub stock_total: usize,
    pub hits: Vec<BackfillHit>,
    pub no_hits: Vec<BackfillNoHit>,
    pub skipped: Vec<BackfillSkip>,
}

#[derive(Debug, Default, Serialize)]
pub struct BackfillApplied {
    pub quarantined: usize,
    pub trust_backfilled: usize,
}

#[derive(Debug, Serialize)]
pub struct BackfillReport {
    pub dry_run: bool,
    pub plan_digest: String,
    #[serde(flatten)]
    pub plan: BackfillPlan,
    pub applied: Option<BackfillApplied>,
}

impl BackfillPlan {
    /// Digest the complete, ordered rehearsal plan so an explicit apply can
    /// be bound to the exact dry-run output the operator reviewed.
    pub(crate) fn digest(&self) -> String {
        let payload =
            serde_json::to_vec(&(self.stock_total, &self.hits, &self.no_hits, &self.skipped))
                .expect("BackfillPlan is serializable");
        sha256_hex(&payload)
    }
}

fn sha256_hex(payload: &[u8]) -> String {
    Sha256::digest(payload)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Load the pre-v076 stock: Dream-written active memories whose trust class
/// still carries the v060 default. Post-v076 merges are stamped
/// `external_content` by `mark_dream_generated` and never match this query,
/// which also makes repeated runs idempotent.
fn load_stock(conn: &Connection) -> Result<Vec<StockMemory>> {
    let mut stmt = conn.prepare(
        "SELECT id, project, topic_key, memory_type, title, content, version,
                updated_at_epoch
         FROM memories
         WHERE session_id = ?1
           AND status = 'active'
           AND source_trust_class = ?2
         ORDER BY project, id",
    )?;
    let rows = stmt.query_map(params![STOCK_SESSION_ID, STOCK_TRUST_CLASS], |row| {
        Ok(StockMemory {
            id: row.get(0)?,
            project: row.get(1)?,
            topic_key: row.get(2)?,
            memory_type: row.get(3)?,
            title: row.get(4)?,
            content: row.get(5)?,
            version: row.get(6)?,
            updated_at_epoch: row.get(7)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

/// Scan one stock row with the exact forward-path convention: individual
/// fields in declared order first, then the combined title/content surface.
fn scan_stock(row: &StockMemory) -> Option<SurfacePatternMatch> {
    let review_text = format!("{}\n{}", row.title, row.content);
    scan_generated_surfaces(&[
        ("dream.topic_key", row.topic_key.as_deref()),
        ("dream.memory_type", Some(row.memory_type.as_str())),
        ("dream.title", Some(row.title.as_str())),
        ("dream.content", Some(row.content.as_str())),
    ])
    .or_else(|| scan_generated_surfaces(&[("dream.title_content", Some(review_text.as_str()))]))
}

pub(crate) fn plan_backfill(conn: &Connection) -> Result<BackfillPlan> {
    let stock = load_stock(conn)?;
    let mut plan = BackfillPlan {
        stock_total: stock.len(),
        ..BackfillPlan::default()
    };
    for row in &stock {
        let Some(matched) = scan_stock(row) else {
            plan.no_hits.push(BackfillNoHit {
                memory_id: row.id,
                project: row.project.clone(),
                snapshot_sha256: stock_snapshot_sha256(row),
            });
            continue;
        };
        if !row.quarantinable() {
            plan.skipped.push(BackfillSkip {
                memory_id: row.id,
                project: row.project.clone(),
                reason: "empty generated title/content cannot satisfy the merge artifact CHECK"
                    .to_string(),
            });
            continue;
        }
        plan.hits.push(BackfillHit {
            memory_id: row.id,
            project: row.project.clone(),
            title: row.title.clone(),
            snapshot_sha256: stock_snapshot_sha256(row),
            matched_field: matched.field.clone(),
            pattern_id: matched.pattern.pattern_id.to_string(),
            pattern_version: matched.pattern.pattern_set_version,
        });
    }
    Ok(plan)
}

fn stock_snapshot_sha256(row: &StockMemory) -> String {
    let payload = serde_json::to_vec(&(
        "gh990-dream-stock-v1",
        row.id,
        row.project.as_str(),
        row.topic_key.as_deref(),
        row.memory_type.as_str(),
        row.title.as_str(),
        row.content.as_str(),
        row.version,
        row.updated_at_epoch,
    ))
    .expect("StockMemory snapshot is serializable");
    sha256_hex(&payload)
}

pub(crate) fn apply_backfill(
    conn: &mut Connection,
    plan: &BackfillPlan,
) -> Result<BackfillApplied> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("begin Dream backfill apply transaction")?;
    let current_plan = plan_backfill(&tx).context("rehearse Dream backfill inside apply")?;
    if current_plan.digest() != plan.digest() {
        bail!("dream_backfill_plan_drift");
    }
    let mut applied = BackfillApplied::default();
    for hit in &plan.hits {
        quarantine_stock_hit(&tx, hit)
            .with_context(|| format!("quarantine stock Dream memory id={}", hit.memory_id))?;
        applied.quarantined += 1;
    }
    for no_hit in &plan.no_hits {
        backfill_trust_class(&tx, no_hit)
            .with_context(|| format!("backfill trust class for memory id={}", no_hit.memory_id))?;
        applied.trust_backfilled += 1;
    }
    tx.commit()
        .context("commit Dream backfill apply transaction")?;
    Ok(applied)
}

pub(crate) fn run_backfill_with_expected_plan_digest(
    conn: &mut Connection,
    dry_run: bool,
    expected_plan_digest: Option<&str>,
) -> Result<BackfillReport> {
    let plan = plan_backfill(conn)?;
    let plan_digest = plan.digest();
    if let Some(expected) = expected_plan_digest
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if expected != plan_digest {
            bail!("dream_backfill_plan_digest_mismatch");
        }
    }
    let applied = if dry_run {
        None
    } else {
        Some(apply_backfill(conn, &plan)?)
    };
    Ok(BackfillReport {
        dry_run,
        plan_digest,
        plan,
        applied,
    })
}

/// Re-load one stock row inside the write transaction and prove it is
/// unchanged since planning; the quarantine ledger is irreversible, so a
/// stale snapshot must abort rather than write a wrong binding.
fn load_stock_for_update(tx: &rusqlite::Transaction, memory_id: i64) -> Result<StockMemory> {
    tx.query_row(
        "SELECT id, project, topic_key, memory_type, title, content, version,
                updated_at_epoch
         FROM memories
         WHERE id = ?1
           AND session_id = ?2
           AND status = 'active'
           AND source_trust_class = ?3",
        params![memory_id, STOCK_SESSION_ID, STOCK_TRUST_CLASS],
        |row| {
            Ok(StockMemory {
                id: row.get(0)?,
                project: row.get(1)?,
                topic_key: row.get(2)?,
                memory_type: row.get(3)?,
                title: row.get(4)?,
                content: row.get(5)?,
                version: row.get(6)?,
                updated_at_epoch: row.get(7)?,
            })
        },
    )
    .with_context(|| {
        format!("stock Dream memory id={memory_id} changed or left the active stock set")
    })
}

fn quarantine_stock_hit(tx: &rusqlite::Transaction, hit: &BackfillHit) -> Result<()> {
    let row = load_stock_for_update(tx, hit.memory_id)?;
    if stock_snapshot_sha256(&row) != hit.snapshot_sha256 {
        bail!("dream_backfill_plan_drift");
    }
    let matched = scan_stock(&row).context("stock row no longer matches the scanner")?;
    if matched.field != hit.matched_field
        || matched.pattern.pattern_id != hit.pattern_id
        || matched.pattern.pattern_set_version != hit.pattern_version
    {
        bail!("stock row scanner verdict changed since planning");
    }

    let topic_key = row.effective_topic_key();
    let member_ids = vec![row.id];
    let intended_superseded_ids = vec![row.id];
    let member_snapshot = crate::dream::DreamClusterMemberSnapshot {
        id: row.id,
        version: row.version,
        updated_at_epoch: row.updated_at_epoch,
        topic_key: row.topic_key.clone(),
        title: row.title.clone(),
        content: row.content.clone(),
    };
    let cluster_signature = crate::dream::cluster_signature_sha256(
        &row.project,
        &row.memory_type,
        std::slice::from_ref(&member_snapshot),
    );
    let decision_payload_sha256 =
        crate::dream::decision_payload_sha256(crate::dream::DreamDecisionPayload::Merge {
            topic_key: &topic_key,
            memory_type: &row.memory_type,
            title: &row.title,
            content: &row.content,
            intended_superseded_ids: &intended_superseded_ids,
        });
    let semantic_discriminator_sha256 = crate::dream::quarantine_semantic_discriminator_sha256(
        &cluster_signature,
        &decision_payload_sha256,
        &matched.field,
        matched.pattern.pattern_id,
        matched.pattern.pattern_set_version,
    );

    let project_id = crate::db::capture::ensure_project_row(tx, &row.project)
        .context("resolve Dream backfill project")?;
    let review_text = format!("{}\n{}", row.title, row.content);
    let outcome = insert_external_candidate(
        tx,
        &ExternalCandidateInsert {
            project_id,
            source_project: &row.project,
            scope: "project",
            memory_type: &row.memory_type,
            topic_key: &topic_key,
            text: &review_text,
            confidence: 0.5,
            risk_class: "high",
            source_kind: SOURCE_KIND,
            semantic_discriminator_sha256: Some(&semantic_discriminator_sha256),
            owner_scope: "repo",
            owner_key: &row.project,
            target_project: Some(&row.project),
            context_class: "startup_core",
            routing_reason: "Dream model-generated consolidation requires explicit review",
            quarantine_match: Some(matched.pattern),
        },
    )
    .context("insert Dream backfill review candidate")?;
    let candidate_id = match outcome {
        ExternalCandidateOutcome::Inserted {
            candidate_id,
            quarantined: true,
        }
        | ExternalCandidateOutcome::Duplicate { candidate_id } => candidate_id,
        ExternalCandidateOutcome::Inserted {
            quarantined: false, ..
        } => bail!("Dream backfill match produced a non-quarantined candidate"),
    };

    let member_ids_json = serde_json::to_string(&member_ids)?;
    let decision_ids_json = serde_json::to_string(&intended_superseded_ids)?;
    let intended_superseded_ids_json = serde_json::to_string(&intended_superseded_ids)?;
    let now = chrono::Utc::now().timestamp();
    let inserted = tx
        .execute(
            "INSERT INTO dream_quarantine_artifacts
         (project, cluster_signature, member_ids_json, source_candidate_id,
          decision_kind, decision_ids_json, decision_payload_sha256,
          intended_superseded_ids_json, generated_topic_key,
          generated_memory_type, generated_title, generated_content,
          generated_field, pattern_id, pattern_version, source_operation,
          source_trust_class, backfill_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'merge', ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                 ?12, ?13, ?14, 'dream', 'external_content', ?15, ?16, ?16)",
            params![
                row.project,
                cluster_signature,
                member_ids_json,
                candidate_id,
                decision_ids_json,
                decision_payload_sha256,
                intended_superseded_ids_json,
                topic_key,
                row.memory_type,
                row.title,
                row.content,
                matched.field,
                matched.pattern.pattern_id,
                matched.pattern.pattern_set_version,
                row.id,
                now,
            ],
        )
        .context("persist Dream backfill quarantine artifact")?;
    if inserted != 1 {
        bail!("dream_backfill_artifact_insert_lost_atomicity");
    }
    let artifact_id: i64 = tx.query_row(
        "SELECT id FROM dream_quarantine_artifacts
         WHERE project = ?1 AND cluster_signature = ?2 AND source_candidate_id = ?3",
        params![row.project, cluster_signature, candidate_id],
        |r| r.get(0),
    )?;

    let retired = tx.execute(
        "UPDATE memories
         SET status = 'archived', source_trust_class = ?3, updated_at_epoch = ?2
         WHERE id = ?1 AND status = 'active'",
        params![row.id, now, DREAM_TRUST_CLASS],
    )?;
    if retired != 1 {
        bail!("dream_backfill_retire_lost_atomicity");
    }
    insert_backfill_operation_log(
        tx,
        "dream_backfill_quarantine",
        &row,
        Some(candidate_id),
        &format!(
            "artifact_id={artifact_id} candidate_id={candidate_id} field={} pattern={}@v{}",
            matched.field, matched.pattern.pattern_id, matched.pattern.pattern_set_version
        ),
        now,
    )?;
    Ok(())
}

/// Backfill the trust class only. `updated_at_epoch` is deliberately left
/// untouched so a maintenance pass cannot make old memories look freshly
/// written to recency-sensitive ranking; the version trigger still bumps the
/// row version because `source_trust_class` is part of the audited column set.
fn backfill_trust_class(tx: &rusqlite::Transaction, no_hit: &BackfillNoHit) -> Result<()> {
    let row = load_stock_for_update(tx, no_hit.memory_id)?;
    if stock_snapshot_sha256(&row) != no_hit.snapshot_sha256 {
        bail!("dream_backfill_plan_drift");
    }
    if scan_stock(&row).is_some() {
        bail!("stock row scanner verdict changed since planning");
    }
    let changed = tx.execute(
        "UPDATE memories SET source_trust_class = ?2
         WHERE id = ?1 AND status = 'active' AND source_trust_class = ?3",
        params![no_hit.memory_id, DREAM_TRUST_CLASS, STOCK_TRUST_CLASS],
    )?;
    if changed != 1 {
        bail!("dream_backfill_trust_class_lost_atomicity");
    }
    let now = chrono::Utc::now().timestamp();
    insert_backfill_operation_log(
        tx,
        "dream_backfill_trust_class",
        &row,
        None,
        "source_trust_class local_tool_output -> external_content",
        now,
    )?;
    Ok(())
}

fn insert_backfill_operation_log(
    conn: &Connection,
    operation: &str,
    row: &StockMemory,
    candidate_id: Option<i64>,
    reason: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_operation_log
         (operation, planner_version, actor, source, owner_scope, owner_key,
          memory_type, input_topic_key, source_candidate_id, result_memory_id,
          reason, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'repo', ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            operation,
            BACKFILL_PLANNER_VERSION,
            BACKFILL_ACTOR,
            BACKFILL_SOURCE,
            row.project,
            row.memory_type,
            row.topic_key,
            candidate_id,
            row.id,
            reason,
            now,
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
