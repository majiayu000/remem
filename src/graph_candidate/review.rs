use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::{insert_trusted_graph_edge, mark_candidate_promoted, ParsedGraphCandidate};

const REVIEWABLE_STATUS_SQL: &str = "c.review_status IN ('pending_review', 'deferred')";
const REVIEWABLE_STATUS_LABEL: &str = "pending_review or deferred";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewGraphCandidate {
    pub id: i64,
    pub project: Option<String>,
    pub candidate_type: String,
    pub edge_type: String,
    pub from_ref: String,
    pub to_ref: String,
    pub evidence_event_ids: Vec<i64>,
    pub evidence_preview: Vec<String>,
    pub confidence: f64,
    pub risk_class: String,
    pub reason: String,
    pub review_status: String,
    pub promoted_edge_id: Option<i64>,
    pub created_at_epoch: i64,
}

#[derive(Debug, Clone)]
struct GraphCandidateRow {
    id: i64,
    project_id: Option<i64>,
    project: Option<String>,
    source_project: String,
    candidate_type: String,
    edge_type: String,
    from_ref: String,
    to_ref: String,
    evidence_event_ids: String,
    confidence: f64,
    risk_class: String,
    reason: String,
    review_status: String,
    promoted_edge_id: Option<i64>,
    created_at_epoch: i64,
}

pub(crate) fn list_pending(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<ReviewGraphCandidate>> {
    load_reviewable_rows(conn, project, limit)
}

pub(crate) fn inspect_candidate(
    conn: &Connection,
    id: i64,
) -> Result<Option<ReviewGraphCandidate>> {
    load_row(conn, id)?
        .map(|row| row.into_review(conn))
        .transpose()
}

pub(crate) fn approve_candidate(conn: &mut Connection, id: i64) -> Result<Option<i64>> {
    let tx = conn.transaction()?;
    let Some(row) = load_row(&tx, id)? else {
        return Ok(None);
    };
    ensure_reviewable(&row)?;
    let candidate = row.as_candidate()?;
    let project_id = row
        .project_id
        .with_context(|| format!("graph candidate {} is missing project_id", row.id))?;
    let outcome = insert_trusted_graph_edge(
        &tx,
        &row.source_project,
        project_id,
        row.id,
        &candidate,
        "graph_review",
    )?;
    mark_candidate_promoted(&tx, row.id, "approved", &outcome)?;
    tx.commit()?;
    Ok(Some(outcome.edge_id))
}

pub(crate) fn reject_candidate(conn: &Connection, id: i64, reason: &str) -> Result<bool> {
    update_review_status(conn, id, "rejected", reason)
}

pub(crate) fn defer_candidate(conn: &Connection, id: i64, reason: &str) -> Result<bool> {
    update_review_status(conn, id, "deferred", reason)
}

fn load_row(conn: &Connection, id: i64) -> Result<Option<GraphCandidateRow>> {
    conn.query_row(
        "SELECT c.id, c.project_id, p.project_path, c.source_project, c.candidate_type, c.edge_type,
                c.from_ref, c.to_ref, c.evidence_event_ids, c.confidence, c.risk_class,
                c.reason, c.review_status, c.promoted_edge_id, c.created_at_epoch
         FROM graph_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE c.id = ?1",
        params![id],
        GraphCandidateRow::from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn ensure_reviewable(row: &GraphCandidateRow) -> Result<()> {
    if !matches!(row.review_status.as_str(), "pending_review" | "deferred") {
        bail!(
            "graph candidate {} is {}, expected {}",
            row.id,
            row.review_status,
            REVIEWABLE_STATUS_LABEL
        );
    }
    Ok(())
}

fn update_review_status(conn: &Connection, id: i64, status: &str, reason: &str) -> Result<bool> {
    let reason = reason.trim();
    if reason.is_empty() {
        bail!("{status} requires a reason");
    }
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE graph_candidates
         SET review_status = ?1,
             review_note = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4
           AND review_status IN ('pending_review', 'deferred')",
        params![status, reason, now, id],
    )?;
    Ok(updated > 0)
}

fn load_reviewable_rows(
    conn: &Connection,
    project: Option<&str>,
    limit: i64,
) -> Result<Vec<ReviewGraphCandidate>> {
    let limit = limit.clamp(1, 200);
    let mut sql = String::from(
        "SELECT c.id, c.project_id, p.project_path, c.source_project, c.candidate_type, c.edge_type,
                c.from_ref, c.to_ref, c.evidence_event_ids, c.confidence, c.risk_class,
                c.reason, c.review_status, c.promoted_edge_id, c.created_at_epoch
         FROM graph_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE ",
    );
    if project.is_some() {
        sql.push_str("p.project_path = ?1 AND ");
    }
    sql.push_str(REVIEWABLE_STATUS_SQL);
    sql.push_str(" ORDER BY c.created_at_epoch ASC, c.id ASC LIMIT ");
    sql.push_str(&limit.to_string());

    let mut stmt = conn.prepare(&sql)?;
    let rows = if let Some(project) = project {
        stmt.query_map(params![project], GraphCandidateRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?
    } else {
        stmt.query_map([], GraphCandidateRow::from_row)?
            .collect::<Result<Vec<_>, _>>()?
    };
    rows.into_iter().map(|row| row.into_review(conn)).collect()
}

impl GraphCandidateRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            project_id: row.get(1)?,
            project: row.get(2)?,
            source_project: row.get(3)?,
            candidate_type: row.get(4)?,
            edge_type: row.get(5)?,
            from_ref: row.get(6)?,
            to_ref: row.get(7)?,
            evidence_event_ids: row.get(8)?,
            confidence: row.get(9)?,
            risk_class: row.get(10)?,
            reason: row.get(11)?,
            review_status: row.get(12)?,
            promoted_edge_id: row.get(13)?,
            created_at_epoch: row.get(14)?,
        })
    }

    fn as_candidate(&self) -> Result<ParsedGraphCandidate> {
        Ok(ParsedGraphCandidate {
            candidate_type: self.candidate_type.clone(),
            edge_type: self.edge_type.clone(),
            from_ref: self.from_ref.clone(),
            to_ref: self.to_ref.clone(),
            evidence_event_ids: serde_json::from_str(&self.evidence_event_ids)
                .with_context(|| format!("graph candidate {} has malformed evidence", self.id))?,
            confidence: self.confidence,
            risk_class: self.risk_class.clone(),
            reason: self.reason.clone(),
        })
    }

    fn into_review(self, conn: &Connection) -> Result<ReviewGraphCandidate> {
        let evidence_event_ids: Vec<i64> = serde_json::from_str(&self.evidence_event_ids)
            .with_context(|| format!("graph candidate {} has malformed evidence", self.id))?;
        Ok(ReviewGraphCandidate {
            evidence_preview: evidence_preview(conn, &evidence_event_ids)?,
            evidence_event_ids,
            id: self.id,
            project: self.project,
            candidate_type: self.candidate_type,
            edge_type: self.edge_type,
            from_ref: self.from_ref,
            to_ref: self.to_ref,
            confidence: self.confidence,
            risk_class: self.risk_class,
            reason: self.reason,
            review_status: self.review_status,
            promoted_edge_id: self.promoted_edge_id,
            created_at_epoch: self.created_at_epoch,
        })
    }
}

fn evidence_preview(conn: &Connection, event_ids: &[i64]) -> Result<Vec<String>> {
    let mut previews = Vec::new();
    for event_id in event_ids.iter().copied().take(3) {
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
