use std::collections::HashSet;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rusqlite::{params, Connection, OptionalExtension};

use super::super::helpers::error_response;
use super::super::types::{
    CandidateDetailItem, CandidateDetailResponse, CandidateEvidenceItem, CandidateReviewDecision,
    DbState,
};

pub(super) struct CandidateDetailProjection {
    pub response: CandidateDetailResponse,
}

pub(in crate::api) async fn handle_candidate_detail(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let conn = match crate::db::open_db() {
        Ok(conn) => conn,
        Err(_) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "candidate_detail_failed",
                "candidate detail could not be evaluated safely",
            )
            .into_response()
        }
    };
    match load_candidate_detail(&conn, id) {
        Ok(Some(projection)) => Json(projection.response).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &format!("candidate {id} not found"),
        )
        .into_response(),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "candidate_detail_failed",
            "candidate detail could not be evaluated safely",
        )
        .into_response(),
    }
}

pub(super) fn load_candidate_detail(
    conn: &Connection,
    id: i64,
) -> Result<Option<CandidateDetailProjection>> {
    let row = conn
        .query_row(
            "SELECT c.id, c.project_id, p.project_path, c.scope, c.memory_type,
                    c.topic_key, c.text, c.source_kind, c.source_project,
                    c.target_project, c.owner_scope, c.owner_key, c.topic_domain,
                    c.routing_confidence, c.routing_reason, c.context_class,
                    c.confidence, c.risk_class, c.review_status,
                    c.auto_promote_block_reason, c.source_trust_class,
                    c.quarantine_pattern_id, c.quarantine_pattern_version,
                    c.version, c.created_at_epoch, c.updated_at_epoch,
                    c.evidence_event_ids
             FROM memory_candidates c
             LEFT JOIN projects p ON p.id = c.project_id
             WHERE c.id = ?1",
            params![id],
            |row| {
                Ok((
                    CandidateDetailItem {
                        id: row.get(0)?,
                        project: row.get(2)?,
                        scope: row.get(3)?,
                        memory_type: row.get(4)?,
                        topic_key: row.get(5)?,
                        text: row.get(6)?,
                        source_kind: row.get(7)?,
                        source_project: row.get(8)?,
                        target_project: row.get(9)?,
                        owner_scope: row.get(10)?,
                        owner_key: row.get(11)?,
                        topic_domain: row.get(12)?,
                        routing_confidence: row.get(13)?,
                        routing_reason: row.get(14)?,
                        context_class: row.get(15)?,
                        confidence: row.get(16)?,
                        risk_class: row.get(17)?,
                        review_status: row.get(18)?,
                        auto_promote_block_reason: row.get(19)?,
                        source_trust_class: row.get(20)?,
                        quarantine_pattern_id: row.get(21)?,
                        quarantine_pattern_version: row.get(22)?,
                        version: row.get(23)?,
                        created_at_epoch: row.get(24)?,
                        updated_at_epoch: row.get(25)?,
                    },
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, String>(26)?,
                ))
            },
        )
        .optional()
        .context("load candidate detail")?;
    let Some((mut candidate, project_id, evidence_json)) = row else {
        return Ok(None);
    };

    let suppressions = load_active_suppressions(conn)?;
    let candidate_suppressed = candidate_is_suppressed(&candidate, &suppressions);
    redact_candidate_fields(&mut candidate);
    let mut blocked = Vec::new();
    if !matches!(
        candidate.review_status.as_str(),
        "pending_review" | "quarantined"
    ) {
        push_blocked_reason(&mut blocked, "candidate_not_reviewable");
    }
    if project_id.is_none() {
        push_blocked_reason(&mut blocked, "candidate_project_unavailable");
    }
    if candidate_suppressed {
        push_blocked_reason(&mut blocked, "candidate_policy_suppressed");
    }

    let evidence = load_evidence(
        conn,
        project_id,
        &evidence_json,
        &suppressions,
        &mut blocked,
    )?;
    Ok(Some(CandidateDetailProjection {
        response: CandidateDetailResponse {
            data: candidate,
            evidence,
            decision: CandidateReviewDecision {
                can_review: blocked.is_empty(),
                blocked_reasons: blocked,
            },
        },
    }))
}

#[derive(Debug)]
struct ActiveSuppression {
    kind: String,
    target_value: Option<String>,
}

fn load_active_suppressions(conn: &Connection) -> Result<Vec<ActiveSuppression>> {
    let mut stmt = conn.prepare(
        "SELECT target_kind, target_value
         FROM memory_suppressions WHERE status = 'active' ORDER BY id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(ActiveSuppression {
            kind: row.get(0)?,
            target_value: row.get(1)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn candidate_is_suppressed(
    candidate: &CandidateDetailItem,
    suppressions: &[ActiveSuppression],
) -> bool {
    suppressions
        .iter()
        .any(|suppression| match suppression.kind.as_str() {
            "topic_key" => {
                suppression.target_value.as_deref() == Some(candidate.topic_key.as_str())
            }
            "pattern" => suppression.target_value.as_deref().is_some_and(|pattern| {
                candidate_text_contains_pattern(&candidate.text, pattern)
                    || candidate_text_contains_pattern(&candidate.topic_key, pattern)
            }),
            _ => false,
        })
}

fn load_evidence(
    conn: &Connection,
    candidate_project_id: Option<i64>,
    evidence_json: &str,
    suppressions: &[ActiveSuppression],
    blocked: &mut Vec<String>,
) -> Result<Vec<CandidateEvidenceItem>> {
    let ids = match parse_candidate_evidence_ids(evidence_json) {
        Ok(ids) => ids,
        Err(reason) => {
            push_blocked_reason(blocked, reason);
            return Ok(Vec::new());
        }
    };
    if ids.is_empty() {
        push_blocked_reason(blocked, "evidence_required");
        return Ok(Vec::new());
    }
    let mut evidence = Vec::with_capacity(ids.len());
    for id in ids {
        let row = conn
            .query_row(
                "SELECT project_id, event_type, role, tool_name, created_at_epoch
                 FROM captured_events WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((event_project_id, event_type, role, tool_name, created_at_epoch)) = row else {
            push_blocked_reason(blocked, "evidence_missing");
            evidence.push(unavailable_evidence(id, "missing"));
            continue;
        };
        if candidate_project_id != Some(event_project_id) {
            push_blocked_reason(blocked, "evidence_cross_project");
            evidence.push(unavailable_evidence(id, "cross_project"));
            continue;
        }
        let Some((event_type_label, summary)) = safe_event_summary(&event_type) else {
            push_blocked_reason(blocked, "evidence_safe_projection_unavailable");
            evidence.push(CandidateEvidenceItem {
                source_kind: "captured_event",
                source_id: id,
                event_type: safe_event_type(&event_type),
                role: safe_role(role.as_deref()),
                tool_name: safe_tool_name(tool_name.as_deref()),
                created_at_epoch: Some(created_at_epoch),
                summary: String::new(),
                preview: String::new(),
                provenance_status: "unsafe_projection".to_string(),
                redacted: true,
            });
            continue;
        };
        if suppression_matches_text(suppressions, summary) {
            push_blocked_reason(blocked, "evidence_policy_suppressed");
            evidence.push(unavailable_evidence(id, "suppressed"));
            continue;
        }
        evidence.push(CandidateEvidenceItem {
            source_kind: "captured_event",
            source_id: id,
            event_type: Some(event_type_label.to_string()),
            role: safe_role(role.as_deref()),
            tool_name: safe_tool_name(tool_name.as_deref()),
            created_at_epoch: Some(created_at_epoch),
            summary: summary.to_string(),
            preview: String::new(),
            provenance_status: "verified".to_string(),
            redacted: true,
        });
    }
    Ok(evidence)
}

fn parse_candidate_evidence_ids(raw: &str) -> std::result::Result<Vec<i64>, &'static str> {
    let values: Vec<serde_json::Value> =
        serde_json::from_str(raw).map_err(|_| "evidence_ids_invalid")?;
    let mut seen = HashSet::new();
    let mut ids = Vec::with_capacity(values.len());
    for value in values {
        let id = value
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or("evidence_id_invalid")?;
        if !seen.insert(id) {
            return Err("evidence_id_duplicate");
        }
        ids.push(id);
    }
    Ok(ids)
}

fn safe_event_summary(event_type: &str) -> Option<(&'static str, &'static str)> {
    match event_type {
        "file_edit" => Some(("file_edit", "File edit evidence")),
        "file_create" => Some(("file_create", "File creation evidence")),
        "search" => Some(("search", "Search evidence")),
        "bash" => Some(("bash", "Shell command evidence")),
        _ => None,
    }
}

fn safe_event_type(value: &str) -> Option<String> {
    matches!(
        value,
        "file_edit" | "file_create" | "search" | "bash" | "session_stop"
    )
    .then(|| value.to_string())
}

fn safe_role(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| matches!(*value, "user" | "assistant" | "tool" | "system"))
        .map(str::to_string)
}

fn safe_tool_name(value: Option<&str>) -> Option<String> {
    value
        .filter(|value| {
            matches!(
                *value,
                "Edit" | "Write" | "NotebookEdit" | "Bash" | "Grep" | "Glob"
            )
        })
        .map(str::to_string)
}

fn unavailable_evidence(id: i64, status: &str) -> CandidateEvidenceItem {
    CandidateEvidenceItem {
        source_kind: "captured_event",
        source_id: id,
        event_type: None,
        role: None,
        tool_name: None,
        created_at_epoch: None,
        summary: String::new(),
        preview: String::new(),
        provenance_status: status.to_string(),
        redacted: true,
    }
}

fn suppression_matches_text(suppressions: &[ActiveSuppression], text: &str) -> bool {
    suppressions.iter().any(|suppression| {
        suppression.kind == "pattern"
            && suppression
                .target_value
                .as_deref()
                .is_some_and(|pattern| candidate_text_contains_pattern(text, pattern))
    })
}

fn candidate_text_contains_pattern(text: &str, pattern: &str) -> bool {
    text.to_lowercase().contains(&pattern.to_lowercase())
}

fn redact_candidate_fields(candidate: &mut CandidateDetailItem) {
    candidate.text = crate::adapter::common::redact_sensitive_text(&candidate.text);
    candidate.routing_reason = candidate
        .routing_reason
        .take()
        .map(|value| crate::adapter::common::redact_sensitive_text(&value));
}

fn push_blocked_reason(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}
