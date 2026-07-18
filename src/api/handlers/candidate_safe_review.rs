use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::{params, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};

use crate::api::mutation::{
    insert_mutation, lookup_mutation, mutation_request_hash, validate_idempotency_key,
    CredentialFreeMutationBody, MutationIdentity, MutationLookup, NewMutationRecord,
    MUTATION_RESPONSE_SCHEMA_VERSION,
};
use crate::memory_candidate::review::{
    approve_candidate_in_transaction, discard_candidate_with_meta, edit_candidate_in_transaction,
    normalize_candidate_edit, CandidateEdit, ReviewMeta,
};

use super::super::types::{
    CandidateSafeApproveRequest, CandidateSafeEditRequest, CandidateSafeRejectRequest,
    CandidateSafeReviewResponse, DbState, SafeMutationErrorDetail, SafeMutationErrorResponse,
};
use super::candidate_detail::load_candidate_detail;

const RESOURCE_KIND: &str = "candidate";
const REVIEW_ACTOR: &str = "api";

pub(in crate::api) async fn handle_safe_approve_candidate(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Response {
    parse_safe_request(&body, SafeActionKind::Approve)
        .and_then(|(identity, request)| execute_safe_review(id, identity, request))
        .unwrap_or_else(|response| response)
}

pub(in crate::api) async fn handle_safe_reject_candidate(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Response {
    parse_safe_request(&body, SafeActionKind::Reject)
        .and_then(|(identity, request)| execute_safe_review(id, identity, request))
        .unwrap_or_else(|response| response)
}

pub(in crate::api) async fn handle_safe_edit_candidate(
    State(_state): State<DbState>,
    Path(id): Path<i64>,
    body: Bytes,
) -> Response {
    parse_safe_request(&body, SafeActionKind::Edit)
        .and_then(|(identity, request)| execute_safe_review(id, identity, request))
        .unwrap_or_else(|response| response)
}

#[derive(Clone, Copy)]
enum SafeActionKind {
    Approve,
    Reject,
    Edit,
}

enum SafeRequest {
    Approve(CandidateSafeApproveRequest),
    Reject(CandidateSafeRejectRequest),
    Edit(CandidateSafeEditRequest),
}

impl SafeRequest {
    fn mutation_action(&self) -> &'static str {
        match self {
            Self::Approve(_) => "approve",
            Self::Reject(_) => "reject",
            Self::Edit(_) => "edit",
        }
    }

    fn expected_version(&self) -> i64 {
        match self {
            Self::Approve(request) => request.expected_version,
            Self::Reject(request) => request.expected_version,
            Self::Edit(request) => request.expected_version,
        }
    }

    fn reason(&self) -> &str {
        match self {
            Self::Approve(request) => &request.reason,
            Self::Reject(request) => &request.reason,
            Self::Edit(request) => &request.reason,
        }
    }

    fn set_idempotency_key(&mut self, value: String) {
        match self {
            Self::Approve(request) => request.idempotency_key = value,
            Self::Reject(request) => request.idempotency_key = value,
            Self::Edit(request) => request.idempotency_key = value,
        }
    }

    fn request_hash(&self, id: i64) -> anyhow::Result<String> {
        match self {
            Self::Approve(request) => mutation_request_hash(
                RESOURCE_KIND,
                id,
                self.mutation_action(),
                &SafeApproveHashBody {
                    reason: &request.reason,
                    expected_version: request.expected_version,
                    acknowledge_pattern: request.acknowledge_pattern.as_deref(),
                },
            ),
            Self::Reject(request) => mutation_request_hash(
                RESOURCE_KIND,
                id,
                self.mutation_action(),
                &SafeRejectHashBody {
                    reason: &request.reason,
                    expected_version: request.expected_version,
                },
            ),
            Self::Edit(request) => mutation_request_hash(
                RESOURCE_KIND,
                id,
                self.mutation_action(),
                &SafeEditHashBody {
                    reason: &request.reason,
                    expected_version: request.expected_version,
                    scope: request.scope.as_deref(),
                    memory_type: request.memory_type.as_deref(),
                    topic_key: request.topic_key.as_deref(),
                    text: request.text.as_deref(),
                },
            ),
        }
    }
}

#[derive(Serialize)]
struct SafeApproveHashBody<'a> {
    reason: &'a str,
    expected_version: i64,
    acknowledge_pattern: Option<&'a str>,
}

impl CredentialFreeMutationBody for SafeApproveHashBody<'_> {}

#[derive(Serialize)]
struct SafeRejectHashBody<'a> {
    reason: &'a str,
    expected_version: i64,
}

impl CredentialFreeMutationBody for SafeRejectHashBody<'_> {}

#[derive(Serialize)]
struct SafeEditHashBody<'a> {
    reason: &'a str,
    expected_version: i64,
    scope: Option<&'a str>,
    memory_type: Option<&'a str>,
    topic_key: Option<&'a str>,
    text: Option<&'a str>,
}

impl CredentialFreeMutationBody for SafeEditHashBody<'_> {}

fn parse_safe_request(
    body: &[u8],
    kind: SafeActionKind,
) -> Result<(MutationIdentity, SafeRequest), Response> {
    let value: Value = serde_json::from_slice(body).map_err(|_| {
        safe_error(
            StatusCode::BAD_REQUEST,
            "candidate_review_request_invalid",
            "request body must be valid JSON",
            None,
        )
    })?;
    let raw_key = value
        .as_object()
        .and_then(|object| object.get("idempotency_key"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            safe_error(
                StatusCode::BAD_REQUEST,
                "idempotency_key_invalid",
                "idempotency key is required",
                None,
            )
        })?;
    let identity = validate_idempotency_key(&raw_key).map_err(|_| {
        safe_error(
            StatusCode::BAD_REQUEST,
            "idempotency_key_invalid",
            "idempotency key is invalid",
            None,
        )
    })?;
    let operation_id = Some(identity.operation_id.as_str());
    let mut request = match kind {
        SafeActionKind::Approve => serde_json::from_value(value)
            .map(SafeRequest::Approve)
            .map_err(|_| invalid_typed_request(operation_id))?,
        SafeActionKind::Reject => serde_json::from_value(value)
            .map(SafeRequest::Reject)
            .map_err(|_| invalid_typed_request(operation_id))?,
        SafeActionKind::Edit => serde_json::from_value(value)
            .map(SafeRequest::Edit)
            .map_err(|_| invalid_typed_request(operation_id))?,
    };
    request.set_idempotency_key(raw_key.trim().to_string());
    validate_safe_request(&mut request, operation_id)?;
    Ok((identity, request))
}

fn validate_safe_request(
    request: &mut SafeRequest,
    operation_id: Option<&str>,
) -> Result<(), Response> {
    let reason = request.reason().trim().to_string();
    if reason.is_empty() || reason.len() > 1024 {
        return Err(safe_error(
            StatusCode::BAD_REQUEST,
            "reason_invalid",
            "reason must contain between 1 and 1024 UTF-8 bytes",
            operation_id,
        ));
    }
    if request.expected_version() < 0 {
        return Err(invalid_typed_request(operation_id));
    }
    match request {
        SafeRequest::Approve(request) => {
            request.reason = reason;
            request.acknowledge_pattern = request
                .acknowledge_pattern
                .take()
                .map(|value| value.trim().to_string());
        }
        SafeRequest::Reject(request) => request.reason = reason,
        SafeRequest::Edit(request) => {
            let edit = normalize_candidate_edit(CandidateEdit {
                scope: request.scope.take(),
                memory_type: request.memory_type.take(),
                topic_key: request.topic_key.take(),
                text: request.text.take(),
            })
            .map_err(|_| {
                safe_error(
                    StatusCode::BAD_REQUEST,
                    "candidate_review_request_invalid",
                    "edit fields are invalid",
                    operation_id,
                )
            })?;
            request.reason = reason;
            request.scope = edit.scope;
            request.memory_type = edit.memory_type;
            request.topic_key = edit.topic_key;
            request.text = edit.text;
        }
    }
    Ok(())
}

fn execute_safe_review(
    id: i64,
    identity: MutationIdentity,
    request: SafeRequest,
) -> Result<Response, Response> {
    if id <= 0 {
        return Err(invalid_typed_request(Some(&identity.operation_id)));
    }
    let mut conn = crate::db::open_db()
        .map_err(|_| safe_internal_error("db_open_failed", Some(&identity.operation_id)))?;
    execute_safe_review_on_connection(&mut conn, id, identity, request)
}

fn execute_safe_review_on_connection(
    conn: &mut rusqlite::Connection,
    id: i64,
    identity: MutationIdentity,
    request: SafeRequest,
) -> Result<Response, Response> {
    let request_hash = request.request_hash(id).map_err(|_| {
        safe_internal_error("candidate_review_hash_failed", Some(&identity.operation_id))
    })?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| {
            safe_internal_error(
                "candidate_review_transaction_failed",
                Some(&identity.operation_id),
            )
        })?;

    match lookup_mutation(&tx, &identity, &request_hash).map_err(|_| {
        safe_internal_error(
            "candidate_review_ledger_failed",
            Some(&identity.operation_id),
        )
    })? {
        MutationLookup::Replay(stored) => {
            let mut response: CandidateSafeReviewResponse =
                serde_json::from_str(&stored.response_json).map_err(|_| {
                    safe_internal_error(
                        "candidate_review_replay_invalid",
                        Some(&identity.operation_id),
                    )
                })?;
            response.replayed = true;
            tx.commit().map_err(|_| {
                safe_internal_error(
                    "candidate_review_transaction_failed",
                    Some(&identity.operation_id),
                )
            })?;
            return Ok(Json(response).into_response());
        }
        MutationLookup::Conflict => {
            return Err(safe_error(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                "idempotency key was already used for a different request",
                Some(&identity.operation_id),
            ));
        }
        MutationLookup::UnsupportedSchema(_) => {
            return Err(safe_error(
                StatusCode::CONFLICT,
                "idempotency_schema_unsupported",
                "stored response schema is not supported",
                Some(&identity.operation_id),
            ));
        }
        MutationLookup::Miss => {}
    }

    let projection = load_candidate_detail(&tx, id)
        .map_err(|_| {
            safe_internal_error(
                "candidate_review_evaluation_failed",
                Some(&identity.operation_id),
            )
        })?
        .ok_or_else(|| {
            safe_error(
                StatusCode::NOT_FOUND,
                "candidate_not_found",
                "candidate was not found",
                Some(&identity.operation_id),
            )
        })?;
    if !projection.response.decision.can_review {
        let code = if projection
            .response
            .decision
            .blocked_reasons
            .iter()
            .any(|reason| reason == "candidate_not_reviewable")
        {
            "candidate_not_reviewable"
        } else {
            "evidence_blocked"
        };
        return Err(safe_error(
            StatusCode::CONFLICT,
            code,
            "candidate cannot be reviewed safely",
            Some(&identity.operation_id),
        ));
    }
    let before_status = projection.response.data.review_status.clone();
    if projection.response.data.version != request.expected_version() {
        return Err(safe_error(
            StatusCode::CONFLICT,
            "version_conflict",
            "candidate version does not match expected_version",
            Some(&identity.operation_id),
        ));
    }
    let project = projection.response.data.project.clone().ok_or_else(|| {
        safe_internal_error(
            "candidate_review_project_unavailable",
            Some(&identity.operation_id),
        )
    })?;
    let mut meta = ReviewMeta::single(REVIEW_ACTOR);
    meta.reason = Some(request.reason().to_string());
    let memory_id = apply_safe_action(&tx, id, &request, &meta).map_err(|_| {
        safe_error(
            StatusCode::CONFLICT,
            "candidate_review_rejected",
            "candidate review action was rejected",
            Some(&identity.operation_id),
        )
    })?;
    let (after_status, version): (String, i64) = tx
        .query_row(
            "SELECT review_status, version FROM memory_candidates WHERE id = ?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| {
            safe_internal_error(
                "candidate_review_state_failed",
                Some(&identity.operation_id),
            )
        })?;
    let occurred_at_epoch = chrono::Utc::now().timestamp();
    let audit_detail = json!({
        "action": request.mutation_action(),
        "after_status": after_status,
        "before_status": before_status,
        "operation_id": identity.operation_id,
        "reason": request.reason(),
    })
    .to_string();
    tx.execute(
        "INSERT INTO events(session_id, project, event_type, summary, detail, created_at_epoch)
         VALUES (?1, ?2, 'candidate_review', ?3, ?4, ?5)",
        params![
            format!("api:{}", identity.operation_id),
            project,
            format!("Candidate review {}", request.mutation_action()),
            audit_detail,
            occurred_at_epoch
        ],
    )
    .map_err(|_| {
        safe_internal_error(
            "candidate_review_audit_failed",
            Some(&identity.operation_id),
        )
    })?;
    let audit_id = tx.last_insert_rowid();
    let response = CandidateSafeReviewResponse {
        response_schema_version: MUTATION_RESPONSE_SCHEMA_VERSION,
        operation_id: identity.operation_id.clone(),
        audit_id,
        candidate_id: id,
        memory_id,
        action: request.mutation_action().to_string(),
        before_status,
        after_status,
        version,
        occurred_at_epoch,
        replayed: false,
    };
    let response_json = serde_json::to_string(&response).map_err(|_| {
        safe_internal_error(
            "candidate_review_response_failed",
            Some(&identity.operation_id),
        )
    })?;
    insert_mutation(
        &tx,
        &NewMutationRecord {
            identity: &identity,
            request_hash: &request_hash,
            resource_kind: RESOURCE_KIND,
            resource_id: id,
            action: request.mutation_action(),
            response_json: &response_json,
            audit_id,
            created_at_epoch: occurred_at_epoch,
        },
    )
    .map_err(|_| {
        safe_internal_error(
            "candidate_review_ledger_failed",
            Some(&identity.operation_id),
        )
    })?;
    tx.commit().map_err(|_| {
        safe_internal_error(
            "candidate_review_transaction_failed",
            Some(&identity.operation_id),
        )
    })?;
    Ok(Json(response).into_response())
}

#[cfg(test)]
pub(in crate::api) fn execute_safe_review_for_test(
    conn: &mut rusqlite::Connection,
    id: i64,
    action: &str,
    body: &[u8],
) -> Response {
    let kind = match action {
        "approve" => SafeActionKind::Approve,
        "reject" => SafeActionKind::Reject,
        "edit" => SafeActionKind::Edit,
        _ => return invalid_typed_request(None),
    };
    parse_safe_request(body, kind)
        .and_then(|(identity, request)| {
            execute_safe_review_on_connection(conn, id, identity, request)
        })
        .unwrap_or_else(|response| response)
}

fn apply_safe_action(
    conn: &rusqlite::Connection,
    id: i64,
    request: &SafeRequest,
    meta: &ReviewMeta,
) -> anyhow::Result<Option<i64>> {
    match request {
        SafeRequest::Approve(request) => {
            approve_candidate_in_transaction(conn, id, meta, request.acknowledge_pattern.as_deref())
        }
        SafeRequest::Reject(_) => {
            anyhow::ensure!(
                discard_candidate_with_meta(conn, id, meta)?,
                "candidate changed state"
            );
            Ok(None)
        }
        SafeRequest::Edit(request) => edit_candidate_in_transaction(
            conn,
            id,
            CandidateEdit {
                scope: request.scope.clone(),
                memory_type: request.memory_type.clone(),
                topic_key: request.topic_key.clone(),
                text: request.text.clone(),
            },
            meta,
        ),
    }
}

fn invalid_typed_request(operation_id: Option<&str>) -> Response {
    safe_error(
        StatusCode::BAD_REQUEST,
        "candidate_review_request_invalid",
        "request fields are invalid",
        operation_id,
    )
}

fn safe_internal_error(code: &str, operation_id: Option<&str>) -> Response {
    safe_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        code,
        "candidate review could not be completed",
        operation_id,
    )
}

fn safe_error(
    status: StatusCode,
    code: &str,
    message: &str,
    operation_id: Option<&str>,
) -> Response {
    (
        status,
        Json(SafeMutationErrorResponse {
            error: SafeMutationErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
                operation_id: operation_id.map(str::to_string),
            },
        }),
    )
        .into_response()
}
