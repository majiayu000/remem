use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::runtime_config::AutoPromotePolicy;

use super::claims::{
    load_claim, UserContextClaim, UserContextClaimType, UserContextSensitivity, DEFAULT_OWNER_KEY,
    DEFAULT_OWNER_SCOPE, DEFAULT_USER_KEY,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserContextCandidateRisk {
    Low,
    Medium,
    High,
}

impl UserContextCandidateRisk {
    pub fn db_value(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UserContextCandidate {
    pub id: i64,
    pub user_key: String,
    pub owner_scope: String,
    pub owner_key: String,
    pub source_project: Option<String>,
    pub host: Option<String>,
    pub session_id: Option<String>,
    pub claim_type: String,
    pub claim_key: Option<String>,
    pub claim_text: String,
    pub confidence: f64,
    pub sensitivity: String,
    pub risk_class: String,
    pub source_kind: String,
    pub source_refs_json: String,
    pub source_preview: Option<String>,
    pub review_status: String,
    pub auto_promote_block_reason: Option<String>,
    pub review_note: Option<String>,
    pub result_claim_id: Option<i64>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
}

#[derive(Debug, Clone)]
pub struct CandidateCreateRequest<'a> {
    pub text: &'a str,
    pub owner_scope: Option<&'a str>,
    pub owner_key: Option<&'a str>,
    pub source_project: Option<&'a str>,
    pub host: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub claim_type: UserContextClaimType,
    pub claim_key: Option<&'a str>,
    pub confidence: f64,
    pub sensitivity: UserContextSensitivity,
    pub risk_class: UserContextCandidateRisk,
    pub source_kind: &'a str,
    pub source_refs_json: &'a str,
    pub source_preview: Option<&'a str>,
    pub auto_promote: bool,
    pub auto_promote_block_reason: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct CandidateListRequest<'a> {
    pub review_status: Option<&'a str>,
    pub include_resolved: bool,
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub struct CandidateEditRequest<'a> {
    pub text: &'a str,
    pub claim_type: Option<UserContextClaimType>,
    pub claim_key: Option<&'a str>,
    pub sensitivity: Option<UserContextSensitivity>,
    pub review_note: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CandidateApplyResult {
    pub candidate: UserContextCandidate,
    pub claim: Option<UserContextClaim>,
    pub action: String,
}

pub fn create_candidate(
    conn: &Connection,
    req: &CandidateCreateRequest<'_>,
) -> Result<CandidateApplyResult> {
    create_candidate_with_policy(conn, req, &AutoPromotePolicy::relaxed_default())
}

pub(crate) fn create_candidate_with_policy(
    conn: &Connection,
    req: &CandidateCreateRequest<'_>,
    policy: &AutoPromotePolicy,
) -> Result<CandidateApplyResult> {
    let text = normalize_required("candidate text", req.text)?;
    validate_confidence(req.confidence)?;
    validate_source_refs(req.source_refs_json)?;
    let (owner_scope, owner_key) = normalized_owner(req.owner_scope, req.owner_key)?;
    let claim_type = req.claim_type.db_value();
    let claim_key = normalized_optional(req.claim_key);
    let source_kind = normalize_required("source kind", req.source_kind)?;
    let source_project = normalized_optional(req.source_project);
    let host = normalized_optional(req.host);
    let session_id = normalized_optional(req.session_id);
    let source_preview = normalized_optional(req.source_preview);
    if let Some(reason) =
        crate::user_context::non_retention::block_reason(text, source_preview, source_kind)
    {
        bail!("user-context candidate blocked by non-retention policy: {reason}");
    }
    let now = chrono::Utc::now().timestamp();
    let allowed = auto_promote_allowed(req, source_kind, policy);
    let block_reason = if allowed {
        None
    } else {
        Some(
            normalized_optional(req.auto_promote_block_reason)
                .unwrap_or_else(|| auto_promote_block_reason(req, source_kind, policy))
                .to_string(),
        )
    };

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO user_context_candidates
         (user_key, owner_scope, owner_key, source_project, host, session_id,
          claim_type, claim_key, claim_text, confidence, sensitivity, risk_class,
          source_kind, source_refs_json, source_preview, review_status,
          auto_promote_block_reason, review_note, result_claim_id,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                 ?13, ?14, ?15, 'pending_review', ?16, NULL, NULL, ?17, ?17)",
        params![
            DEFAULT_USER_KEY,
            owner_scope,
            owner_key,
            source_project,
            host,
            session_id,
            claim_type,
            claim_key,
            text,
            req.confidence,
            req.sensitivity.db_value(),
            req.risk_class.db_value(),
            source_kind,
            req.source_refs_json,
            source_preview,
            block_reason,
            now,
        ],
    )
    .context("insert user-context candidate")?;
    let id = tx.last_insert_rowid();
    let result = if allowed {
        apply_candidate_tx(&tx, id, None, "auto_promoted")?
    } else {
        CandidateApplyResult {
            candidate: load_candidate_tx(&tx, id)?,
            claim: None,
            action: "pending_review".to_string(),
        }
    };
    tx.commit()?;
    Ok(result)
}

pub fn list_candidates(
    conn: &Connection,
    req: &CandidateListRequest<'_>,
) -> Result<Vec<UserContextCandidate>> {
    let mut conditions = Vec::new();
    let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    if let Some(status) = normalized_optional(req.review_status) {
        validate_review_status(status)?;
        conditions.push(format!("review_status = ?{idx}"));
        values.push(Box::new(status.to_string()));
        idx += 1;
    } else if !req.include_resolved {
        conditions.push("review_status IN ('pending_review', 'deferred')".to_string());
    }
    let mut sql = candidate_select_sql().to_string();
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    sql.push_str(&format!(
        " ORDER BY updated_at_epoch DESC, id DESC LIMIT ?{idx}"
    ));
    values.push(Box::new(req.limit.clamp(1, 500)));
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&values);
    let rows = stmt.query_map(refs.as_slice(), candidate_from_row)?;
    crate::db::query::collect_rows(rows)
}

pub fn load_candidate(conn: &Connection, id: i64) -> Result<UserContextCandidate> {
    load_candidate_tx(conn, id)
}

pub fn approve_candidate(conn: &Connection, id: i64) -> Result<CandidateApplyResult> {
    let tx = conn.unchecked_transaction()?;
    let result = apply_candidate_tx(&tx, id, None, "approved")?;
    tx.commit()?;
    Ok(result)
}

pub fn edit_candidate(
    conn: &Connection,
    id: i64,
    req: &CandidateEditRequest<'_>,
) -> Result<CandidateApplyResult> {
    let tx = conn.unchecked_transaction()?;
    let result = apply_candidate_tx(&tx, id, Some(req), "edited")?;
    tx.commit()?;
    Ok(result)
}

pub fn reject_candidate(
    conn: &Connection,
    id: i64,
    review_note: Option<&str>,
) -> Result<UserContextCandidate> {
    transition_candidate(conn, id, "rejected", review_note)
}

pub fn suppress_candidate(
    conn: &Connection,
    id: i64,
    review_note: Option<&str>,
) -> Result<UserContextCandidate> {
    transition_candidate(conn, id, "suppressed", review_note)
}

fn apply_candidate_tx(
    conn: &Connection,
    id: i64,
    edit: Option<&CandidateEditRequest<'_>>,
    final_status: &str,
) -> Result<CandidateApplyResult> {
    let mut candidate = load_candidate_tx(conn, id)?;
    ensure_reviewable(&candidate)?;
    let claim_type = edit
        .and_then(|edit| edit.claim_type)
        .map(UserContextClaimType::db_value)
        .unwrap_or(candidate.claim_type.as_str())
        .to_string();
    let text = edit
        .map(|edit| normalize_required("candidate text", edit.text))
        .transpose()?
        .unwrap_or(candidate.claim_text.as_str())
        .to_string();
    let sensitivity = edit
        .and_then(|edit| edit.sensitivity)
        .map(UserContextSensitivity::db_value)
        .unwrap_or(candidate.sensitivity.as_str())
        .to_string();
    let claim_key = edit
        .and_then(|edit| normalized_optional(edit.claim_key))
        .map(str::to_string)
        .or_else(|| candidate.claim_key.clone())
        .ok_or_else(|| {
            anyhow!(
                "claim_key is required before applying user-context candidate {}",
                candidate.id
            )
        })?;
    let note = edit
        .and_then(|edit| normalized_optional(edit.review_note))
        .map(str::to_string);
    if let Some(reason) = crate::user_context::non_retention::block_reason(
        &text,
        candidate.source_preview.as_deref(),
        &candidate.source_kind,
    ) {
        bail!("user-context candidate blocked by non-retention policy: {reason}");
    }
    let now = chrono::Utc::now().timestamp();
    if edit.is_some() {
        let updated = conn.execute(
            "UPDATE user_context_candidates
             SET claim_type = ?1,
                 claim_key = ?2,
                 claim_text = ?3,
                 sensitivity = ?4,
                 review_note = ?5,
                 updated_at_epoch = ?6
             WHERE id = ?7
               AND review_status IN ('pending_review', 'deferred')",
            params![&claim_type, &claim_key, &text, &sensitivity, note, now, id],
        )?;
        if updated != 1 {
            bail!("user-context candidate {id} is no longer reviewable");
        }
        candidate = load_candidate_tx(conn, id)?;
    }
    let active = active_claims_for_key(
        conn,
        &candidate.owner_scope,
        &candidate.owner_key,
        &claim_type,
        &claim_key,
    )?;
    if final_status == "auto_promoted" && active_claim_key_conflict(&active, &text, &sensitivity) {
        block_candidate_auto_promote(
            conn,
            id,
            "claim_key_conflict_requires_review",
            note.as_deref(),
            now,
        )?;
        return Ok(CandidateApplyResult {
            candidate: load_candidate_tx(conn, id)?,
            claim: None,
            action: "pending_review".to_string(),
        });
    }
    if let Some(existing) = active
        .iter()
        .find(|claim| claim.claim_text == text && claim.sensitivity == sensitivity)
    {
        supersede_other_active_claims(conn, &active, existing.id, now)?;
        update_candidate_after_apply(
            conn,
            id,
            final_status,
            existing.id,
            note.as_deref()
                .or(Some("noop: existing active claim matches candidate")),
            now,
        )?;
        return Ok(CandidateApplyResult {
            candidate: load_candidate_tx(conn, id)?,
            claim: Some(existing.clone()),
            action: "noop_existing_claim".to_string(),
        });
    }
    for claim in &active {
        conn.execute(
            "UPDATE user_context_claims
             SET status = 'superseded', updated_at_epoch = ?1
             WHERE id = ?2",
            params![now, claim.id],
        )?;
    }
    let supersedes_claim_id = active.first().map(|claim| claim.id);
    let source_refs_json = claim_source_refs_json(&candidate)?;
    conn.execute(
        "INSERT INTO user_context_claims
         (user_key, owner_scope, owner_key, claim_type, claim_key, claim_text,
          confidence, sensitivity, source_kind, source_refs_json, status,
          valid_from_epoch, valid_to_epoch, last_confirmed_at_epoch,
          supersedes_claim_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'user_context_candidate',
                 ?9, 'active', NULL, NULL, ?10, ?11, ?10, ?10)",
        params![
            candidate.user_key,
            candidate.owner_scope,
            candidate.owner_key,
            &claim_type,
            &claim_key,
            &text,
            candidate.confidence,
            &sensitivity,
            source_refs_json,
            now,
            supersedes_claim_id,
        ],
    )?;
    let claim = load_claim(conn, conn.last_insert_rowid())?;
    update_candidate_after_apply(conn, id, final_status, claim.id, note.as_deref(), now)?;
    Ok(CandidateApplyResult {
        candidate: load_candidate_tx(conn, id)?,
        claim: Some(claim),
        action: if supersedes_claim_id.is_some() {
            "superseded_existing_claim".to_string()
        } else {
            "created_claim".to_string()
        },
    })
}

fn transition_candidate(
    conn: &Connection,
    id: i64,
    review_status: &str,
    review_note: Option<&str>,
) -> Result<UserContextCandidate> {
    let candidate = load_candidate(conn, id)?;
    ensure_reviewable(&candidate)?;
    let note = normalized_optional(review_note);
    let updated = conn.execute(
        "UPDATE user_context_candidates
         SET review_status = ?1, review_note = ?2, updated_at_epoch = ?3
         WHERE id = ?4
           AND review_status IN ('pending_review', 'deferred')",
        params![review_status, note, chrono::Utc::now().timestamp(), id],
    )?;
    if updated != 1 {
        bail!("user-context candidate {id} is no longer reviewable");
    }
    load_candidate(conn, id)
}

fn active_claims_for_key(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    claim_type: &str,
    claim_key: &str,
) -> Result<Vec<UserContextClaim>> {
    let mut stmt = conn.prepare(
        "SELECT id, user_key, owner_scope, owner_key, claim_type, claim_key,
                claim_text, confidence, sensitivity, source_kind,
                source_refs_json, status, valid_from_epoch, valid_to_epoch,
                last_confirmed_at_epoch, supersedes_claim_id,
                created_at_epoch, updated_at_epoch
         FROM user_context_claims
         WHERE owner_scope = ?1
           AND owner_key = ?2
           AND claim_type = ?3
           AND claim_key = ?4
           AND status = 'active'
         ORDER BY updated_at_epoch DESC, id DESC",
    )?;
    let rows = stmt.query_map(
        params![owner_scope, owner_key, claim_type, claim_key],
        |row| {
            Ok(UserContextClaim {
                id: row.get(0)?,
                user_key: row.get(1)?,
                owner_scope: row.get(2)?,
                owner_key: row.get(3)?,
                claim_type: row.get(4)?,
                claim_key: row.get(5)?,
                claim_text: row.get(6)?,
                confidence: row.get(7)?,
                sensitivity: row.get(8)?,
                source_kind: row.get(9)?,
                source_refs_json: row.get(10)?,
                status: row.get(11)?,
                valid_from_epoch: row.get(12)?,
                valid_to_epoch: row.get(13)?,
                last_confirmed_at_epoch: row.get(14)?,
                supersedes_claim_id: row.get(15)?,
                created_at_epoch: row.get(16)?,
                updated_at_epoch: row.get(17)?,
            })
        },
    )?;
    crate::db::query::collect_rows(rows)
}

fn supersede_other_active_claims(
    conn: &Connection,
    active: &[UserContextClaim],
    keep_id: i64,
    now: i64,
) -> Result<()> {
    for claim in active.iter().filter(|claim| claim.id != keep_id) {
        conn.execute(
            "UPDATE user_context_claims
             SET status = 'superseded', updated_at_epoch = ?1
             WHERE id = ?2",
            params![now, claim.id],
        )?;
    }
    Ok(())
}

fn block_candidate_auto_promote(
    conn: &Connection,
    id: i64,
    reason: &str,
    review_note: Option<&str>,
    now: i64,
) -> Result<()> {
    let updated = conn.execute(
        "UPDATE user_context_candidates
         SET auto_promote_block_reason = ?1,
             review_note = ?2,
             updated_at_epoch = ?3
         WHERE id = ?4
           AND review_status IN ('pending_review', 'deferred')",
        params![reason, review_note, now, id],
    )?;
    if updated != 1 {
        bail!("user-context candidate {id} is no longer reviewable");
    }
    Ok(())
}

fn update_candidate_after_apply(
    conn: &Connection,
    id: i64,
    review_status: &str,
    claim_id: i64,
    review_note: Option<&str>,
    now: i64,
) -> Result<()> {
    let updated = conn.execute(
        "UPDATE user_context_candidates
         SET review_status = ?1, result_claim_id = ?2, review_note = ?3,
             updated_at_epoch = ?4
         WHERE id = ?5
           AND review_status IN ('pending_review', 'deferred')",
        params![review_status, claim_id, review_note, now, id],
    )?;
    if updated != 1 {
        bail!("user-context candidate {id} is no longer reviewable");
    }
    Ok(())
}

fn ensure_reviewable(candidate: &UserContextCandidate) -> Result<()> {
    if matches!(
        candidate.review_status.as_str(),
        "pending_review" | "deferred"
    ) {
        return Ok(());
    }
    bail!(
        "only pending_review or deferred user-context candidates can be reviewed; candidate {} is {}",
        candidate.id,
        candidate.review_status
    );
}

fn load_candidate_tx(conn: &Connection, id: i64) -> Result<UserContextCandidate> {
    conn.query_row(
        &format!("{} WHERE id = ?1", candidate_select_sql()),
        [id],
        candidate_from_row,
    )
    .optional()?
    .ok_or_else(|| anyhow!("user-context candidate {id} not found"))
}

fn candidate_select_sql() -> &'static str {
    "SELECT id, user_key, owner_scope, owner_key, source_project, host,
            session_id, claim_type, claim_key, claim_text, confidence,
            sensitivity, risk_class, source_kind, source_refs_json,
            source_preview, review_status, auto_promote_block_reason,
            review_note, result_claim_id, created_at_epoch, updated_at_epoch
     FROM user_context_candidates"
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<UserContextCandidate> {
    Ok(UserContextCandidate {
        id: row.get(0)?,
        user_key: row.get(1)?,
        owner_scope: row.get(2)?,
        owner_key: row.get(3)?,
        source_project: row.get(4)?,
        host: row.get(5)?,
        session_id: row.get(6)?,
        claim_type: row.get(7)?,
        claim_key: row.get(8)?,
        claim_text: row.get(9)?,
        confidence: row.get(10)?,
        sensitivity: row.get(11)?,
        risk_class: row.get(12)?,
        source_kind: row.get(13)?,
        source_refs_json: row.get(14)?,
        source_preview: row.get(15)?,
        review_status: row.get(16)?,
        auto_promote_block_reason: row.get(17)?,
        review_note: row.get(18)?,
        result_claim_id: row.get(19)?,
        created_at_epoch: row.get(20)?,
        updated_at_epoch: row.get(21)?,
    })
}

fn active_claim_key_conflict(active: &[UserContextClaim], text: &str, sensitivity: &str) -> bool {
    active
        .iter()
        .any(|claim| claim.claim_text != text || claim.sensitivity != sensitivity)
}

fn auto_promote_allowed(
    req: &CandidateCreateRequest<'_>,
    source_kind: &str,
    policy: &AutoPromotePolicy,
) -> bool {
    req.auto_promote
        && req.risk_class == UserContextCandidateRisk::Low
        && req.sensitivity == UserContextSensitivity::Normal
        && req.confidence >= policy.min_confidence
        && policy.allows_source_kind(source_kind)
        && normalized_optional(req.claim_key).is_some()
}

fn auto_promote_block_reason(
    req: &CandidateCreateRequest<'_>,
    source_kind: &str,
    policy: &AutoPromotePolicy,
) -> &'static str {
    if !req.auto_promote {
        return "requires_review";
    }
    if source_kind == "third_party_statement" {
        return "third_party_requires_review";
    }
    if req.risk_class != UserContextCandidateRisk::Low {
        return "risk_requires_review";
    }
    if req.sensitivity != UserContextSensitivity::Normal {
        return "sensitivity_requires_review";
    }
    if req.confidence < policy.min_confidence {
        return "low_confidence";
    }
    if !policy.allows_source_kind(source_kind) {
        return "source_requires_review";
    }
    if normalized_optional(req.claim_key).is_none() {
        return "missing_claim_key";
    }
    "requires_review"
}

fn claim_source_refs_json(candidate: &UserContextCandidate) -> Result<String> {
    let source_refs: serde_json::Value = serde_json::from_str(&candidate.source_refs_json)?;
    serde_json::to_string(&serde_json::json!([{
        "kind": "user_context_candidate",
        "candidate_id": candidate.id,
        "source_kind": candidate.source_kind,
        "source_refs": source_refs,
    }]))
    .context("encode candidate claim source refs")
}

fn validate_review_status(status: &str) -> Result<()> {
    if matches!(
        status,
        "pending_review"
            | "auto_promoted"
            | "approved"
            | "edited"
            | "rejected"
            | "suppressed"
            | "deferred"
    ) {
        return Ok(());
    }
    bail!("unsupported user-context candidate review status: {status}");
}

fn normalized_owner<'a>(
    owner_scope: Option<&'a str>,
    owner_key: Option<&'a str>,
) -> Result<(&'a str, &'a str)> {
    let owner_scope = normalized_optional(owner_scope).unwrap_or(DEFAULT_OWNER_SCOPE);
    validate_owner_scope(owner_scope)?;
    let owner_key = normalized_optional(owner_key);
    match (owner_scope, owner_key) {
        ("user", None) => Ok((owner_scope, DEFAULT_OWNER_KEY)),
        ("user", Some(owner_key)) => Ok((owner_scope, owner_key)),
        (_, Some(owner_key)) => Ok((owner_scope, owner_key)),
        _ => bail!("owner_key is required when owner_scope is not user"),
    }
}

fn validate_owner_scope(owner_scope: &str) -> Result<()> {
    if matches!(owner_scope, "user" | "workspace" | "repo" | "session") {
        return Ok(());
    }
    bail!("unsupported user-context owner scope: {owner_scope}");
}

fn normalize_required<'a>(label: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} cannot be empty");
    }
    Ok(value)
}

fn normalized_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn validate_confidence(confidence: f64) -> Result<()> {
    if (0.0..=1.0).contains(&confidence) {
        return Ok(());
    }
    bail!("confidence must be between 0.0 and 1.0");
}

fn validate_source_refs(source_refs_json: &str) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_str(source_refs_json).context("parse candidate source refs")?;
    if !value.is_array() {
        bail!("candidate source refs must be a JSON array");
    }
    if value.as_array().is_some_and(Vec::is_empty) {
        bail!("candidate source refs must not be empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
