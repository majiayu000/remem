use std::collections::HashSet;

use anyhow::Result;
use rusqlite::{params, Connection};

use super::normalize::{compact_line, relevant_to_request, search_query};
use super::types::{
    ClaimCandidate, NormalizedRequest, RecallCandidate, RecallState, UserRecallDroppedItem,
    MAX_CLAIM_SCAN, MAX_SESSION_SCAN,
};
use crate::user_context::claims::{self, DEFAULT_OWNER_KEY, DEFAULT_OWNER_SCOPE};

pub(super) fn collect_summary(
    conn: &Connection,
    req: &NormalizedRequest,
    state: &mut RecallState,
) -> Result<()> {
    let summary_req = crate::user_context::summary::SummaryRequest {
        owner_scope: Some(&req.owner_scope),
        owner_key: Some(&req.owner_key),
        project: &req.project,
    };
    if let Some(summary) = crate::user_context::summary::load_active_summary(conn, &summary_req)? {
        state.counts.summaries += 1;
        if !relevant_to_request(&summary.summary_text, req) {
            return Ok(());
        }
        state.candidates.push(RecallCandidate {
            source_type: "profile_summary".to_string(),
            source_id: Some(summary.id),
            title: Some(format!("profile summary v{}", summary.version)),
            text: compact_line(&summary.summary_text, 900),
            reason_codes: vec![
                "profile_summary".to_string(),
                "query_match".to_string(),
                "safe_sources".to_string(),
            ],
            source_refs: Some(serde_json::json!({
                "claim_ids": summary.source_claim_ids,
                "memory_ids": summary.source_memory_ids,
                "activity_refs": summary.source_activity_refs,
            })),
            priority: 80,
        });
    }
    Ok(())
}

pub(super) fn collect_claims(
    conn: &Connection,
    req: &NormalizedRequest,
    state: &mut RecallState,
) -> Result<()> {
    let claims = load_claim_candidates(conn, req)?;
    state.counts.claims += claims.len();
    let now = chrono::Utc::now().timestamp();
    for claim in claims {
        let label = Some(format!("{}:{}", claim.claim_type, claim.claim_key));
        if let Some(reason) = recall_claim_drop_reason(conn, &claim, req, now)? {
            state.dropped.push(UserRecallDroppedItem {
                source_type: "user_claim".to_string(),
                source_id: Some(claim.id),
                label,
                reason_code: reason,
            });
            continue;
        }
        if !relevant_to_request(&claim.claim_text, req)
            && !relevant_to_request(&claim.claim_key, req)
            && !relevant_to_request(&claim.claim_type, req)
        {
            state.dropped.push(UserRecallDroppedItem {
                source_type: "user_claim".to_string(),
                source_id: Some(claim.id),
                label,
                reason_code: "not_relevant".to_string(),
            });
            continue;
        }
        let source_refs = serde_json::from_str::<serde_json::Value>(&claim.source_refs_json).ok();
        state.candidates.push(RecallCandidate {
            source_type: "user_claim".to_string(),
            source_id: Some(claim.id),
            title: Some(format!("{}:{}", claim.claim_type, claim.claim_key)),
            text: compact_line(&claim.claim_text, 500),
            reason_codes: vec![
                "active_user_claim".to_string(),
                "query_match".to_string(),
                format!("owner:{}:{}", claim.owner_scope, claim.owner_key),
            ],
            source_refs,
            priority: 100,
        });
    }
    Ok(())
}

pub(super) fn collect_memories(
    conn: &Connection,
    req: &NormalizedRequest,
    state: &mut RecallState,
) -> Result<()> {
    let result = crate::memory::service::search_memories(
        conn,
        &crate::memory::service::SearchRequest {
            query: Some(search_query(req)),
            project: Some(req.project.clone()),
            memory_type: None,
            limit: 5,
            offset: 0,
            include_stale: false,
            include_suppressed: req.include_suppressed,
            branch: None,
            multi_hop: false,
            explain: false,
        },
    )?;
    state.counts.memories += result.memories.len();
    for memory in result.memories {
        if claims::active_preference_backfill_covers_user_preference_memory(conn, memory.id)? {
            state.dropped.push(UserRecallDroppedItem {
                source_type: "memory".to_string(),
                source_id: Some(memory.id),
                label: Some(memory.title),
                reason_code: "backfilled_as_user_claim".to_string(),
            });
            continue;
        }
        state.candidates.push(RecallCandidate {
            source_type: "memory".to_string(),
            source_id: Some(memory.id),
            title: Some(memory.title),
            text: compact_line(&memory.text, 650),
            reason_codes: vec![
                "repo_memory_match".to_string(),
                "search_result".to_string(),
                format!("type:{}", memory.memory_type),
            ],
            source_refs: Some(serde_json::json!({
                "topic_key": memory.topic_key,
                "project": memory.project,
                "status": memory.status,
            })),
            priority: 70,
        });
    }
    Ok(())
}

pub(super) fn collect_current_state(
    conn: &Connection,
    req: &NormalizedRequest,
    state: &mut RecallState,
) -> Result<()> {
    for state_key in &req.state_keys {
        let result = crate::memory::current_state::current_state(
            conn,
            &crate::memory::current_state::CurrentStateRequest {
                state_key: state_key.clone(),
                project: Some(req.project.clone()),
                owner_scope: None,
                owner_key: None,
                memory_type: None,
                as_of_epoch: None,
                include_history: false,
            },
        )?;
        if let Some(current) = result.current {
            state.counts.current_state += 1;
            state.candidates.push(RecallCandidate {
                source_type: "current_state".to_string(),
                source_id: Some(current.id),
                title: Some(format!("current state: {state_key}")),
                text: compact_line(&current.text, 650),
                reason_codes: vec![
                    "current_state_answer".to_string(),
                    format!("state_key:{state_key}"),
                ],
                source_refs: Some(serde_json::json!({
                    "state_key": state_key,
                    "memory_id": current.id,
                    "topic_key": current.topic_key,
                    "status": current.status,
                })),
                priority: 90,
            });
        } else {
            state.dropped.push(UserRecallDroppedItem {
                source_type: "current_state".to_string(),
                source_id: None,
                label: Some(state_key.clone()),
                reason_code: format!("current_state_{}", result.status),
            });
        }
    }
    Ok(())
}

pub(super) fn collect_workstreams(
    conn: &Connection,
    req: &NormalizedRequest,
    state: &mut RecallState,
) -> Result<()> {
    let workstreams = crate::workstream::query_active_workstreams(conn, &req.project)?;
    state.counts.workstreams += workstreams.len();
    for workstream in workstreams {
        let text = [
            workstream.title.as_str(),
            workstream.progress.as_deref().unwrap_or_default(),
            workstream.next_action.as_deref().unwrap_or_default(),
            workstream.blockers.as_deref().unwrap_or_default(),
        ]
        .join(" ");
        if !relevant_to_request(&text, req) {
            continue;
        }
        state.candidates.push(RecallCandidate {
            source_type: "workstream".to_string(),
            source_id: Some(workstream.id),
            title: Some(workstream.title),
            text: compact_line(&text, 500),
            reason_codes: vec!["active_workstream".to_string(), "query_match".to_string()],
            source_refs: Some(serde_json::json!({
                "project": workstream.project,
                "status": workstream.status.as_str(),
                "updated_at_epoch": workstream.updated_at_epoch,
            })),
            priority: 60,
        });
    }
    Ok(())
}

pub(super) fn collect_recent_sessions(
    conn: &Connection,
    req: &NormalizedRequest,
    state: &mut RecallState,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id,
                CASE
                  WHEN request LIKE 'Captured event range %..%' THEN
                    COALESCE(NULLIF(decisions, ''), NULLIF(learned, ''),
                             NULLIF(next_steps, ''), NULLIF(preferences, ''),
                             NULLIF(completed, ''), '')
                  ELSE COALESCE(request, '')
                END AS display_request,
                COALESCE(completed, ''),
                COALESCE(decisions, ''), COALESCE(learned, ''),
                COALESCE(next_steps, ''), COALESCE(preferences, ''),
                created_at_epoch
         FROM session_summaries
         WHERE (session_row_id IS NULL
                OR request NOT LIKE 'Captured event range %..%'
                OR COALESCE(decisions, '') != ''
                OR COALESCE(learned, '') != ''
                OR COALESCE(next_steps, '') != ''
                OR COALESCE(preferences, '') != '')
           AND ((owner_scope = 'repo' AND owner_key = ?1)
             OR (owner_scope = 'repo' AND target_project = ?1)
             OR (owner_scope IS NULL AND project = ?1))
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![req.project, MAX_SESSION_SCAN], |row| {
        Ok(SessionCandidate {
            id: row.get(0)?,
            request: row.get(1)?,
            completed: row.get(2)?,
            decisions: row.get(3)?,
            learned: row.get(4)?,
            next_steps: row.get(5)?,
            preferences: row.get(6)?,
            created_at_epoch: row.get(7)?,
        })
    })?;
    let sessions = crate::db::query::collect_rows(rows)?;
    state.counts.sessions += sessions.len();
    let mut seen_session_text = HashSet::new();
    for session in sessions {
        let text = session.text();
        let dedupe_key = text.to_ascii_lowercase();
        if !seen_session_text.insert(dedupe_key) {
            continue;
        }
        if !relevant_to_request(&text, req) {
            continue;
        }
        state.candidates.push(RecallCandidate {
            source_type: "session_summary".to_string(),
            source_id: Some(session.id),
            title: Some(compact_line(&session.request, 120)),
            text: compact_line(&text, 550),
            reason_codes: vec!["recent_session".to_string(), "query_match".to_string()],
            source_refs: Some(serde_json::json!({
                "project": req.project,
                "created_at_epoch": session.created_at_epoch,
            })),
            priority: 40,
        });
    }
    Ok(())
}

fn load_claim_candidates(
    conn: &Connection,
    req: &NormalizedRequest,
) -> Result<Vec<ClaimCandidate>> {
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    let owner_filter =
        if req.owner_scope == DEFAULT_OWNER_SCOPE && req.owner_key == DEFAULT_OWNER_KEY {
            values.push(Box::new(req.owner_scope.clone()));
            values.push(Box::new(req.owner_key.clone()));
            values.push(Box::new(req.project.clone()));
            idx += 3;
            "((owner_scope = ?1 AND owner_key = ?2) OR (owner_scope = 'repo' AND owner_key = ?3))"
                .to_string()
        } else {
            values.push(Box::new(req.owner_scope.clone()));
            values.push(Box::new(req.owner_key.clone()));
            idx += 2;
            "owner_scope = ?1 AND owner_key = ?2".to_string()
        };

    let sql = format!(
        "SELECT id, claim_type, claim_key, claim_text, owner_scope, owner_key,
                sensitivity, source_refs_json, status, valid_from_epoch, valid_to_epoch
         FROM user_context_claims
         WHERE {owner_filter}
         ORDER BY updated_at_epoch DESC, id DESC
         LIMIT ?{idx}",
    );
    values.push(Box::new(MAX_CLAIM_SCAN));
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&values);
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(ClaimCandidate {
            id: row.get(0)?,
            claim_type: row.get(1)?,
            claim_key: row.get(2)?,
            claim_text: row.get(3)?,
            owner_scope: row.get(4)?,
            owner_key: row.get(5)?,
            sensitivity: row.get(6)?,
            source_refs_json: row.get(7)?,
            status: row.get(8)?,
            valid_from_epoch: row.get(9)?,
            valid_to_epoch: row.get(10)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

fn recall_claim_drop_reason(
    conn: &Connection,
    claim: &ClaimCandidate,
    req: &NormalizedRequest,
    now: i64,
) -> Result<Option<String>> {
    if claim.status == "suppressed" && !req.include_suppressed {
        return Ok(Some("status:suppressed".to_string()));
    }
    if claim.status != "active" && claim.status != "suppressed" {
        return Ok(Some(format!("status:{}", claim.status)));
    }
    if !req.include_sensitive
        && matches!(
            claim.sensitivity.as_str(),
            "personal" | "sensitive" | "restricted"
        )
    {
        return Ok(Some(format!("sensitivity:{}", claim.sensitivity)));
    }
    if claim
        .valid_from_epoch
        .is_some_and(|valid_from| valid_from > now)
    {
        return Ok(Some("not_yet_valid".to_string()));
    }
    if claim.valid_to_epoch.is_some_and(|valid_to| valid_to <= now) {
        return Ok(Some("expired".to_string()));
    }
    if !req.include_suppressed
        && crate::memory::suppression::user_claim_is_policy_suppressed(conn, claim.id)?
    {
        return Ok(Some("policy_suppressed".to_string()));
    }
    Ok(None)
}

struct SessionCandidate {
    id: i64,
    request: String,
    completed: String,
    decisions: String,
    learned: String,
    next_steps: String,
    preferences: String,
    created_at_epoch: i64,
}

impl SessionCandidate {
    fn text(&self) -> String {
        [
            self.request.as_str(),
            self.completed.as_str(),
            self.decisions.as_str(),
            self.learned.as_str(),
            self.next_steps.as_str(),
            self.preferences.as_str(),
        ]
        .join(" ")
    }
}
