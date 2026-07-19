use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rusqlite::TransactionBehavior;
use serde::Serialize;
use serde_json::Value;

use crate::api::mutation::{
    insert_mutation, lookup_mutation, mutation_request_hash, validate_idempotency_key,
    CredentialFreeMutationBody, MutationIdentity, MutationLookup, NewMutationRecord,
    MUTATION_RESPONSE_SCHEMA_VERSION,
};
use crate::memory::governance::{
    govern_memory_for_web_in_transaction, WebMemoryGovernanceAction, WebMemoryGovernanceDecision,
    WebMemoryGovernanceRequest,
};

use super::super::types::{
    DbState, MemorySafeGovernanceRequest, MemorySafeGovernanceResponse, SafeMutationErrorDetail,
    SafeMutationErrorResponse,
};

const RESOURCE_KIND: &str = "memory";
const GOVERNANCE_ACTOR: &str = "api";

struct GovernanceFailure(Box<Response>);

impl From<Response> for GovernanceFailure {
    fn from(response: Response) -> Self {
        Self(Box::new(response))
    }
}

impl GovernanceFailure {
    fn into_response(self) -> Response {
        *self.0
    }
}

type GovernanceResult<T> = Result<T, GovernanceFailure>;

pub(in crate::api) async fn handle_archive_memory(
    State(_state): State<DbState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    parse_request(&body)
        .and_then(|(identity, request)| {
            execute_governance(id, WebMemoryGovernanceAction::Archive, identity, request)
        })
        .unwrap_or_else(GovernanceFailure::into_response)
}

pub(in crate::api) async fn handle_restore_memory(
    State(_state): State<DbState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    parse_request(&body)
        .and_then(|(identity, request)| {
            execute_governance(id, WebMemoryGovernanceAction::Restore, identity, request)
        })
        .unwrap_or_else(GovernanceFailure::into_response)
}

#[derive(Serialize)]
struct GovernanceHashBody<'a> {
    reason: &'a str,
    expected_version: i64,
}

impl CredentialFreeMutationBody for GovernanceHashBody<'_> {}

fn parse_request(body: &[u8]) -> GovernanceResult<(MutationIdentity, MemorySafeGovernanceRequest)> {
    let value: Value = serde_json::from_slice(body).map_err(|_| {
        safe_error(
            StatusCode::BAD_REQUEST,
            "memory_governance_request_invalid",
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
    let mut request: MemorySafeGovernanceRequest = serde_json::from_value(value).map_err(|_| {
        safe_error(
            StatusCode::BAD_REQUEST,
            "memory_governance_request_invalid",
            "request fields are invalid",
            operation_id,
        )
    })?;
    request.reason = request.reason.trim().to_string();
    request.idempotency_key.clear();
    if request.reason.is_empty() || request.reason.len() > 1024 {
        return Err(safe_error(
            StatusCode::BAD_REQUEST,
            "reason_invalid",
            "reason must contain between 1 and 1024 UTF-8 bytes",
            operation_id,
        )
        .into());
    }
    if request.expected_version <= 0 {
        return Err(safe_error(
            StatusCode::BAD_REQUEST,
            "memory_governance_request_invalid",
            "expected_version must be positive",
            operation_id,
        )
        .into());
    }
    Ok((identity, request))
}

fn execute_governance(
    raw_id: String,
    action: WebMemoryGovernanceAction,
    identity: MutationIdentity,
    request: MemorySafeGovernanceRequest,
) -> GovernanceResult<Response> {
    let memory_id = parse_memory_id(&raw_id, &identity)?;
    let mut conn = crate::db::open_db()
        .map_err(|_| safe_internal_error("db_open_failed", Some(&identity.operation_id)))?;
    execute_governance_on_connection(&mut conn, memory_id, action, identity, request)
}

fn parse_memory_id(raw_id: &str, identity: &MutationIdentity) -> GovernanceResult<i64> {
    match raw_id.parse::<i64>() {
        Ok(id) if id > 0 => Ok(id),
        _ => Err(safe_error(
            StatusCode::BAD_REQUEST,
            "id_invalid",
            "memory id must be a positive integer",
            Some(&identity.operation_id),
        )
        .into()),
    }
}

fn execute_governance_on_connection(
    conn: &mut rusqlite::Connection,
    memory_id: i64,
    action: WebMemoryGovernanceAction,
    identity: MutationIdentity,
    request: MemorySafeGovernanceRequest,
) -> GovernanceResult<Response> {
    let request_hash = mutation_request_hash(
        RESOURCE_KIND,
        memory_id,
        action.as_str(),
        &GovernanceHashBody {
            reason: &request.reason,
            expected_version: request.expected_version,
        },
    )
    .map_err(|_| {
        safe_internal_error(
            "memory_governance_hash_failed",
            Some(&identity.operation_id),
        )
    })?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| {
            safe_internal_error(
                "memory_governance_transaction_failed",
                Some(&identity.operation_id),
            )
        })?;

    match lookup_mutation(&tx, &identity, &request_hash).map_err(|_| {
        safe_internal_error(
            "memory_governance_ledger_failed",
            Some(&identity.operation_id),
        )
    })? {
        MutationLookup::Replay(stored) => {
            let mut response: MemorySafeGovernanceResponse =
                serde_json::from_str(&stored.response_json).map_err(|_| {
                    safe_internal_error(
                        "memory_governance_replay_invalid",
                        Some(&identity.operation_id),
                    )
                })?;
            response.replayed = true;
            tx.commit().map_err(|_| {
                safe_internal_error(
                    "memory_governance_transaction_failed",
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
            )
            .into())
        }
        MutationLookup::UnsupportedSchema(_) => {
            return Err(safe_error(
                StatusCode::CONFLICT,
                "idempotency_schema_unsupported",
                "stored response schema is not supported",
                Some(&identity.operation_id),
            )
            .into())
        }
        MutationLookup::Miss => {}
    }

    let decision = govern_memory_for_web_in_transaction(
        &tx,
        &WebMemoryGovernanceRequest {
            memory_id,
            action,
            expected_version: request.expected_version,
            operation_id: &identity.operation_id,
            reason: &request.reason,
            actor: GOVERNANCE_ACTOR,
        },
    )
    .map_err(|_| {
        safe_internal_error(
            "memory_governance_mutation_failed",
            Some(&identity.operation_id),
        )
    })?;
    let applied = match decision {
        WebMemoryGovernanceDecision::Applied(applied) => applied,
        WebMemoryGovernanceDecision::NotFound => {
            return Err(safe_error(
                StatusCode::NOT_FOUND,
                "memory_not_found",
                "memory was not found",
                Some(&identity.operation_id),
            )
            .into())
        }
        WebMemoryGovernanceDecision::VersionConflict => {
            return Err(safe_error(
                StatusCode::CONFLICT,
                "version_conflict",
                "memory version does not match expected_version",
                Some(&identity.operation_id),
            )
            .into())
        }
        WebMemoryGovernanceDecision::NotArchivable => {
            return Err(safe_error(
                StatusCode::CONFLICT,
                "memory_not_archivable",
                "memory is not active and cannot be archived",
                Some(&identity.operation_id),
            )
            .into())
        }
        WebMemoryGovernanceDecision::NotRecoverable => {
            return Err(safe_error(
                StatusCode::NOT_FOUND,
                "memory_not_recoverable",
                "memory does not have current Web archive provenance",
                Some(&identity.operation_id),
            )
            .into())
        }
    };
    let response = MemorySafeGovernanceResponse {
        response_schema_version: MUTATION_RESPONSE_SCHEMA_VERSION,
        operation_id: identity.operation_id.clone(),
        audit_id: applied.audit_id,
        memory_id,
        action: action.as_str().to_string(),
        before_status: applied.before_status,
        after_status: applied.after_status,
        version: applied.version,
        occurred_at_epoch: applied.occurred_at_epoch,
        replayed: false,
    };
    let response_json = serde_json::to_string(&response).map_err(|_| {
        safe_internal_error(
            "memory_governance_response_failed",
            Some(&identity.operation_id),
        )
    })?;
    insert_mutation(
        &tx,
        &NewMutationRecord {
            identity: &identity,
            request_hash: &request_hash,
            resource_kind: RESOURCE_KIND,
            resource_id: memory_id,
            action: action.as_str(),
            response_json: &response_json,
            audit_id: applied.audit_id,
            created_at_epoch: applied.occurred_at_epoch,
        },
    )
    .map_err(|_| {
        safe_internal_error(
            "memory_governance_ledger_failed",
            Some(&identity.operation_id),
        )
    })?;
    tx.commit().map_err(|_| {
        safe_internal_error(
            "memory_governance_transaction_failed",
            Some(&identity.operation_id),
        )
    })?;
    Ok(Json(response).into_response())
}

#[cfg(test)]
pub(in crate::api) fn execute_memory_governance_for_test(
    conn: &mut rusqlite::Connection,
    memory_id: i64,
    action: &str,
    body: &[u8],
) -> Response {
    let action = match action {
        "archive" => WebMemoryGovernanceAction::Archive,
        "restore" => WebMemoryGovernanceAction::Restore,
        _ => {
            return safe_error(
                StatusCode::BAD_REQUEST,
                "action_invalid",
                "action is invalid",
                None,
            )
        }
    };
    parse_request(body)
        .and_then(|(identity, request)| {
            execute_governance_on_connection(conn, memory_id, action, identity, request)
        })
        .unwrap_or_else(GovernanceFailure::into_response)
}

fn safe_internal_error(code: &str, operation_id: Option<&str>) -> Response {
    safe_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        code,
        "memory governance request failed",
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
