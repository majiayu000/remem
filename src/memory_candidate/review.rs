use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{
    normalize_memory_type, normalize_scope, normalize_topic_key, route_candidate, CandidateRoute,
    ParsedMemoryCandidate,
};
use crate::memory::poisoning::scan_instruction_pattern;

mod approval;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewCandidate {
    pub id: i64,
    pub project: Option<String>,
    pub scope: String,
    pub memory_type: String,
    pub topic_key: String,
    pub text: String,
    pub evidence_event_ids: String,
    pub evidence_preview: Vec<String>,
    pub confidence: f64,
    pub risk_class: String,
    pub review_status: String,
    pub quarantine_pattern_id: Option<String>,
    pub quarantine_pattern_version: Option<i64>,
    pub created_at_epoch: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CandidateEdit {
    pub scope: Option<String>,
    pub memory_type: Option<String>,
    pub topic_key: Option<String>,
    pub text: Option<String>,
}

/// Durable per-candidate review provenance (#683): who initiated the outcome
/// and whether it came from a single action or a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewMeta {
    pub actor: String,
    pub action_source: ReviewActionSource,
    pub batch_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewActionSource {
    Single,
    Batch,
}

impl ReviewActionSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Batch => "batch",
        }
    }
}

impl ReviewMeta {
    pub(crate) fn single(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            action_source: ReviewActionSource::Single,
            batch_id: None,
            reason: None,
        }
    }

    pub(crate) fn batch(
        actor: impl Into<String>,
        batch_id: impl Into<String>,
        reason: Option<String>,
    ) -> Self {
        Self {
            actor: actor.into(),
            action_source: ReviewActionSource::Batch,
            batch_id: Some(batch_id.into()),
            reason,
        }
    }
}

pub(crate) fn default_review_actor() -> String {
    std::env::var("REMEM_REVIEW_ACTOR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| std::env::var("USER").ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "cli".to_string())
}

/// Filter set shared by batch preview and batch mutation so the preview can
/// never diverge from what executes.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct BatchFilter {
    pub project: Option<String>,
    pub memory_type: Option<String>,
    pub block_reason: Option<String>,
    pub topic_key: Option<String>,
    pub contains: Option<String>,
    pub min_confidence: Option<f64>,
    pub older_than_days: Option<i64>,
    pub limit: i64,
}

const SECS_PER_DAY: i64 = 86_400;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BatchPreview {
    pub ids: Vec<i64>,
    pub by_type: Vec<(String, i64)>,
    pub by_project: Vec<(String, i64)>,
    pub samples: Vec<BatchSample>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BatchSample {
    pub id: i64,
    pub memory_type: String,
    pub topic_key: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BatchOutcome {
    pub batch_id: String,
    pub processed: Vec<i64>,
    pub promoted_memory_ids: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReviewPromotion {
    memory_id: i64,
    promoted: bool,
}

#[derive(Debug, Clone)]
struct CandidateRow {
    id: i64,
    project: Option<String>,
    source_project: Option<String>,
    target_project: Option<String>,
    owner_scope: Option<String>,
    owner_key: Option<String>,
    topic_domain: Option<String>,
    routing_confidence: Option<f64>,
    routing_reason: Option<String>,
    context_class: Option<String>,
    scope: String,
    memory_type: String,
    topic_key: String,
    text: String,
    source_kind: Option<String>,
    evidence_event_ids: String,
    confidence: f64,
    risk_class: String,
    review_status: String,
    created_at_epoch: i64,
    source_trust_class: String,
    quarantine_pattern_id: Option<String>,
    quarantine_pattern_version: Option<i64>,
}

pub(crate) fn list_pending(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<ReviewCandidate>> {
    let limit = limit.clamp(1, 200);
    let rows = if let Some(project) = project {
        let mut stmt = conn.prepare(
            "SELECT c.id, p.project_path, c.scope, c.memory_type, c.topic_key,
                    c.text, c.evidence_event_ids, c.confidence, c.risk_class,
                    c.review_status, c.created_at_epoch, c.source_project,
                    c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                    c.routing_confidence, c.routing_reason, c.context_class,
                    c.source_kind, c.source_trust_class, c.quarantine_pattern_id,
                    c.quarantine_pattern_version
             FROM memory_candidates c
             LEFT JOIN projects p ON p.id = c.project_id
             WHERE c.review_status IN ('pending_review', 'quarantined')
               AND p.project_path = ?1
             ORDER BY c.created_at_epoch ASC, c.id ASC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![project, limit], CandidateRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    } else {
        let mut stmt = conn.prepare(
            "SELECT c.id, p.project_path, c.scope, c.memory_type, c.topic_key,
                    c.text, c.evidence_event_ids, c.confidence, c.risk_class,
                    c.review_status, c.created_at_epoch, c.source_project,
                    c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                    c.routing_confidence, c.routing_reason, c.context_class,
                    c.source_kind, c.source_trust_class, c.quarantine_pattern_id,
                    c.quarantine_pattern_version
             FROM memory_candidates c
             LEFT JOIN projects p ON p.id = c.project_id
             WHERE c.review_status IN ('pending_review', 'quarantined')
             ORDER BY c.created_at_epoch ASC, c.id ASC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit], CandidateRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    rows.into_iter()
        .map(|row| {
            let evidence_preview = evidence_preview(conn, &row.evidence_event_ids)?;
            Ok(ReviewCandidate {
                id: row.id,
                project: row.project,
                scope: row.scope,
                memory_type: row.memory_type,
                topic_key: row.topic_key,
                text: row.text,
                evidence_event_ids: row.evidence_event_ids,
                evidence_preview,
                confidence: row.confidence,
                risk_class: row.risk_class,
                review_status: row.review_status,
                quarantine_pattern_id: row.quarantine_pattern_id,
                quarantine_pattern_version: row.quarantine_pattern_version,
                created_at_epoch: row.created_at_epoch,
            })
        })
        .collect()
}

pub(crate) fn approve_candidate(conn: &mut Connection, id: i64) -> Result<Option<i64>> {
    approve_candidate_with_meta(conn, id, &ReviewMeta::single(default_review_actor()))
}

pub(crate) fn approve_candidate_with_meta(
    conn: &mut Connection,
    id: i64,
    meta: &ReviewMeta,
) -> Result<Option<i64>> {
    approval::approve_candidate_with_meta_and_ack(conn, id, meta, None)
}

pub(crate) fn approve_candidate_with_ack(
    conn: &mut Connection,
    id: i64,
    acknowledged_pattern_id: &str,
) -> Result<Option<i64>> {
    approval::approve_candidate_with_meta_and_ack(
        conn,
        id,
        &ReviewMeta::single(default_review_actor()),
        Some(acknowledged_pattern_id),
    )
}

pub(crate) fn discard_candidate(conn: &Connection, id: i64) -> Result<bool> {
    discard_candidate_with_meta(conn, id, &ReviewMeta::single(default_review_actor()))
}

pub(crate) fn discard_candidate_with_meta(
    conn: &Connection,
    id: i64,
    meta: &ReviewMeta,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE memory_candidates
         SET review_status = 'discarded', updated_at_epoch = ?1,
             review_actor = ?2, reviewed_at_epoch = ?1,
             review_action_source = ?3, review_batch_id = ?4, review_reason = ?5
         WHERE id = ?6 AND review_status IN ('pending_review', 'quarantined')",
        params![
            now,
            meta.actor,
            meta.action_source.as_str(),
            meta.batch_id,
            meta.reason,
            id
        ],
    )?;
    Ok(updated > 0)
}

pub(crate) fn edit_candidate(
    conn: &mut Connection,
    id: i64,
    edit: CandidateEdit,
) -> Result<Option<i64>> {
    if edit.scope.is_none()
        && edit.memory_type.is_none()
        && edit.topic_key.is_none()
        && edit.text.is_none()
    {
        bail!("edit requires at least one changed field");
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let Some(row) = load_candidate(&tx, id)? else {
        return Ok(None);
    };
    ensure_reviewable(&row)?;
    let edited = row.apply_edit(edit)?;
    if let Some(matched) = scan_instruction_pattern(&edited.text) {
        bail!(
            "edited candidate {} matched instruction-pattern {}@v{}; review and acknowledge the pattern before promotion",
            row.id,
            matched.pattern_id,
            matched.pattern_set_version
        );
    }
    let meta = ReviewMeta::single(default_review_actor());
    let promotion = approval::promote_row(&tx, &row, "edited", Some(&edited), &meta, None)?;
    tx.commit()?;
    Ok(Some(promotion.memory_id))
}

pub(crate) fn resolve_batch(conn: &Connection, filter: &BatchFilter) -> Result<BatchPreview> {
    let ids_with_detail = resolve_batch_rows(conn, filter)?;
    let mut by_type: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut by_project: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    let mut samples = Vec::new();
    let mut ids = Vec::with_capacity(ids_with_detail.len());
    for row in &ids_with_detail {
        ids.push(row.id);
        *by_type.entry(row.memory_type.clone()).or_default() += 1;
        *by_project
            .entry(
                row.project
                    .clone()
                    .unwrap_or_else(|| "<unknown project>".to_string()),
            )
            .or_default() += 1;
        if samples.len() < 5 {
            samples.push(BatchSample {
                id: row.id,
                memory_type: row.memory_type.clone(),
                topic_key: row.topic_key.clone(),
                text: crate::db::truncate_str(&row.text, 120).to_string(),
            });
        }
    }
    Ok(BatchPreview {
        ids,
        by_type: by_type.into_iter().collect(),
        by_project: by_project.into_iter().collect(),
        samples,
    })
}

struct BatchRow {
    id: i64,
    project: Option<String>,
    memory_type: String,
    topic_key: String,
    text: String,
}

fn resolve_batch_rows(conn: &Connection, filter: &BatchFilter) -> Result<Vec<BatchRow>> {
    validate_batch_filter(filter)?;
    let limit = filter.limit;
    let mut sql = String::from(
        "SELECT c.id,
                COALESCE(c.target_project, p.project_path, c.source_project,
                         CASE WHEN c.owner_scope = 'repo' THEN c.owner_key END) AS project,
                c.memory_type, c.topic_key, c.text
         FROM memory_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE c.review_status IN ('pending_review', 'quarantined')",
    );
    let mut args: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    if let Some(project) = &filter.project {
        sql.push_str(
            " AND (p.project_path = ? OR c.source_project = ? OR c.target_project = ?
                   OR (c.owner_scope = 'repo' AND c.owner_key = ?))",
        );
        args.push(Box::new(project.clone()));
        args.push(Box::new(project.clone()));
        args.push(Box::new(project.clone()));
        args.push(Box::new(project.clone()));
    }
    if let Some(memory_type) = &filter.memory_type {
        sql.push_str(" AND c.memory_type = ?");
        args.push(Box::new(memory_type.clone()));
    }
    if let Some(block_reason) = &filter.block_reason {
        sql.push_str(" AND c.auto_promote_block_reason = ?");
        args.push(Box::new(block_reason.clone()));
    }
    if let Some(topic_key) = &filter.topic_key {
        sql.push_str(" AND c.topic_key = ?");
        args.push(Box::new(topic_key.clone()));
    }
    if let Some(contains) = &filter.contains {
        let pattern = like_pattern(contains);
        sql.push_str(" AND (c.text LIKE ? ESCAPE '\\' OR c.topic_key LIKE ? ESCAPE '\\')");
        args.push(Box::new(pattern.clone()));
        args.push(Box::new(pattern));
    }
    if let Some(min_confidence) = filter.min_confidence {
        sql.push_str(" AND c.confidence >= ?");
        args.push(Box::new(min_confidence));
    }
    if let Some(older_than_days) = filter.older_than_days {
        let cutoff = older_than_cutoff(chrono::Utc::now().timestamp(), older_than_days)?;
        sql.push_str(" AND c.created_at_epoch <= ?");
        args.push(Box::new(cutoff));
    }
    sql.push_str(" ORDER BY c.created_at_epoch ASC, c.id ASC LIMIT ?");
    args.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())),
            |row| {
                Ok(BatchRow {
                    id: row.get(0)?,
                    project: row.get(1)?,
                    memory_type: row.get(2)?,
                    topic_key: row.get(3)?,
                    text: row.get(4)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn validate_batch_filter(filter: &BatchFilter) -> Result<()> {
    if filter.limit <= 0 {
        bail!("limit must be positive");
    }
    if let Some(contains) = &filter.contains {
        if contains.trim().is_empty() {
            bail!("contains filter must not be empty");
        }
    }
    if let Some(min_confidence) = filter.min_confidence {
        if !(0.0..=1.0).contains(&min_confidence) {
            bail!("min_confidence must be between 0 and 1");
        }
    }
    if let Some(older_than_days) = filter.older_than_days {
        if older_than_days < 0 {
            bail!("older_than_days must be non-negative");
        }
        older_than_cutoff(chrono::Utc::now().timestamp(), older_than_days)?;
    }
    Ok(())
}

fn older_than_cutoff(now_epoch: i64, older_than_days: i64) -> Result<i64> {
    let age_secs = older_than_days
        .checked_mul(SECS_PER_DAY)
        .context("older_than_days is too large")?;
    now_epoch
        .checked_sub(age_secs)
        .context("older_than_days is too large")
}

fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for ch in query.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

pub(crate) fn new_batch_id() -> String {
    format!(
        "batch-{}-{}",
        chrono::Utc::now().timestamp(),
        std::process::id()
    )
}

/// Approve every candidate in the already previewed id set inside one
/// transaction. Rows are re-checked as pending before mutation, but the id set
/// itself is not re-resolved after user confirmation.
pub(crate) fn approve_batch(
    conn: &mut Connection,
    preview: &BatchPreview,
    meta: &ReviewMeta,
) -> Result<BatchOutcome> {
    let batch_id = meta
        .batch_id
        .clone()
        .unwrap_or_else(|| "batch-unset".to_string());
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut promoted_memory_ids = Vec::with_capacity(preview.ids.len());
    for id in &preview.ids {
        let row = load_candidate(&tx, *id)?
            .with_context(|| format!("candidate {id} disappeared during batch"))?;
        ensure_pending(&row)?;
        let promotion = approval::promote_row(&tx, &row, "approved", None, meta, None)?;
        if promotion.promoted {
            promoted_memory_ids.push(promotion.memory_id);
        }
    }
    tx.commit()?;
    Ok(BatchOutcome {
        batch_id,
        processed: preview.ids.clone(),
        promoted_memory_ids,
    })
}

/// Discard every candidate in the already previewed id set inside one
/// transaction.
pub(crate) fn discard_batch(
    conn: &mut Connection,
    preview: &BatchPreview,
    meta: &ReviewMeta,
) -> Result<BatchOutcome> {
    let batch_id = meta
        .batch_id
        .clone()
        .unwrap_or_else(|| "batch-unset".to_string());
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for id in &preview.ids {
        if !discard_candidate_with_meta(&tx, *id, meta)? {
            bail!("candidate {id} was not pending_review during batch discard");
        }
    }
    tx.commit()?;
    Ok(BatchOutcome {
        batch_id,
        processed: preview.ids.clone(),
        promoted_memory_ids: Vec::new(),
    })
}

impl CandidateRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            project: row.get(1)?,
            scope: row.get(2)?,
            memory_type: row.get(3)?,
            topic_key: row.get(4)?,
            text: row.get(5)?,
            evidence_event_ids: row.get(6)?,
            confidence: row.get(7)?,
            risk_class: row.get(8)?,
            review_status: row.get(9)?,
            created_at_epoch: row.get(10)?,
            source_project: row.get(11)?,
            target_project: row.get(12)?,
            owner_scope: row.get(13)?,
            owner_key: row.get(14)?,
            topic_domain: row.get(15)?,
            routing_confidence: row.get(16)?,
            routing_reason: row.get(17)?,
            context_class: row.get(18)?,
            source_kind: row.get(19)?,
            source_trust_class: row.get(20)?,
            quarantine_pattern_id: row.get(21)?,
            quarantine_pattern_version: row.get(22)?,
        })
    }

    fn as_candidate(&self) -> ParsedMemoryCandidate {
        let (title_override, text) = if self.source_kind.as_deref() == Some("pack") {
            decode_pack_review_text(&self.text)
                .map(|(title, content)| (Some(title), content))
                .unwrap_or((None, self.text.clone()))
        } else {
            (None, self.text.clone())
        };
        ParsedMemoryCandidate {
            scope: self.scope.clone(),
            memory_type: self.memory_type.clone(),
            topic_key: self.topic_key.clone(),
            title_override,
            text,
            confidence: self.confidence,
            risk_class: self.risk_class.clone(),
        }
    }

    fn apply_edit(&self, edit: CandidateEdit) -> Result<ParsedMemoryCandidate> {
        let scope = edit
            .scope
            .as_deref()
            .map(normalize_scope)
            .transpose()?
            .unwrap_or_else(|| self.scope.clone());
        let memory_type = edit
            .memory_type
            .as_deref()
            .map(normalize_memory_type)
            .transpose()?
            .unwrap_or_else(|| self.memory_type.clone());
        let topic_key = edit
            .topic_key
            .as_deref()
            .map(normalize_topic_key)
            .transpose()?
            .unwrap_or_else(|| self.topic_key.clone());
        let text = edit
            .text
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.text.clone());
        Ok(ParsedMemoryCandidate {
            scope,
            memory_type,
            topic_key,
            title_override: None,
            text,
            confidence: self.confidence,
            risk_class: self.risk_class.clone(),
        })
    }

    fn route_for(&self, candidate: &ParsedMemoryCandidate) -> CandidateRoute {
        match (
            self.owner_scope.as_ref(),
            self.owner_key.as_ref(),
            self.routing_confidence,
            self.routing_reason.as_ref(),
            self.context_class.as_ref(),
        ) {
            (
                Some(owner_scope),
                Some(owner_key),
                Some(routing_confidence),
                Some(routing_reason),
                Some(context_class),
            ) => CandidateRoute {
                owner_scope: owner_scope.clone(),
                owner_key: owner_key.clone(),
                target_project: self.target_project.clone(),
                topic_domain: self.topic_domain.clone(),
                routing_confidence,
                routing_reason: routing_reason.clone(),
                context_class: context_class.clone(),
            },
            _ => {
                let project = self
                    .source_project
                    .as_deref()
                    .or(self.project.as_deref())
                    .unwrap_or("<unknown>");
                route_candidate(project, None, candidate, std::iter::empty())
            }
        }
    }
}

fn decode_pack_review_text(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines();
    let title_line = lines.next()?;
    let content_marker = lines.next()?;
    let title = title_line.strip_prefix("pack_title:")?.trim().to_string();
    if content_marker.trim() != "pack_content:" || title.is_empty() {
        return None;
    }
    Some((title, lines.collect::<Vec<_>>().join("\n")))
}

fn load_candidate(conn: &Connection, id: i64) -> Result<Option<CandidateRow>> {
    conn.query_row(
        "SELECT c.id, p.project_path, c.scope, c.memory_type, c.topic_key,
                c.text, c.evidence_event_ids, c.confidence, c.risk_class,
                c.review_status, c.created_at_epoch, c.source_project,
                c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                c.routing_confidence, c.routing_reason, c.context_class,
                c.source_kind, c.source_trust_class, c.quarantine_pattern_id,
                c.quarantine_pattern_version
         FROM memory_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE c.id = ?1",
        params![id],
        CandidateRow::from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn ensure_pending(row: &CandidateRow) -> Result<()> {
    if row.review_status != "pending_review" {
        bail!(
            "candidate {} is {}, expected pending_review",
            row.id,
            row.review_status
        );
    }
    Ok(())
}

fn ensure_reviewable(row: &CandidateRow) -> Result<()> {
    if !matches!(row.review_status.as_str(), "pending_review" | "quarantined") {
        bail!(
            "candidate {} is {}, expected pending_review or quarantined",
            row.id,
            row.review_status
        );
    }
    Ok(())
}

fn evidence_preview(conn: &Connection, evidence_json: &str) -> Result<Vec<String>> {
    let event_ids: Vec<i64> = serde_json::from_str(evidence_json)
        .with_context(|| "candidate has malformed evidence_event_ids")?;
    let mut previews = Vec::new();
    for event_id in event_ids.into_iter().take(3) {
        let row: Option<(String, Option<String>, String)> = conn
            .query_row(
                "SELECT event_type, tool_name, COALESCE(content_text, '')
                 FROM captured_events WHERE id = ?1",
                params![event_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((event_type, tool_name, content)) = row {
            let tool = tool_name
                .map(|value| format!(" tool={value}"))
                .unwrap_or_default();
            previews.push(format!(
                "#{} {}{} {}",
                event_id,
                event_type,
                tool,
                crate::db::truncate_str(&content, 120)
            ));
        } else {
            previews.push(format!("#{event_id} <missing event>"));
        }
    }
    Ok(previews)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod poisoning_tests;
