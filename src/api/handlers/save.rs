use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use crate::memory::service;

use super::super::helpers::{error_response, open_request_db};
use super::super::types::{
    DbState, LocalCopyResponse, SaveMemoryNextStepResponse, SaveMemoryRequest, SaveMemoryResponse,
};

pub(in crate::api) async fn handle_save_memory(
    State(_state): State<DbState>,
    Json(req): Json<SaveMemoryRequest>,
) -> impl IntoResponse {
    let conn = match open_request_db() {
        Ok(conn) => conn,
        Err(response) => return response,
    };

    let reference_time_epoch = req.reference_time_epoch;
    let save_req = service::SaveMemoryRequest {
        text: req.text,
        title: req.title,
        project: req.project,
        session_id: req.session_id,
        host: req.host.or_else(|| Some("api".to_string())),
        topic_key: req.topic_key,
        memory_type: req.memory_type,
        files: req.files,
        scope: req.scope,
        created_at_epoch: req.created_at_epoch,
        branch: req.branch,
        local_path: req.local_path,
        local_copy_enabled: req.local_copy_enabled,
        claim_enabled: req.claim_enabled,
        claim_source: req.claim_source.or_else(|| Some("api_save".to_string())),
    };

    match service::save_memory_with_reference_time(&conn, &save_req, reference_time_epoch) {
        Ok(saved) => (
            StatusCode::CREATED,
            Json(SaveMemoryResponse {
                id: saved.id,
                status: saved.status,
                memory_type: saved.memory_type,
                project: saved.project,
                scope: saved.scope,
                topic_key: saved.topic_key,
                branch: saved.branch,
                operation: saved.operation,
                created_at_epoch: saved.created_at_epoch,
                reference_time_epoch: saved.reference_time_epoch,
                updated_at_epoch: saved.updated_at_epoch,
                upserted: saved.upserted,
                local_copy: LocalCopyResponse {
                    status: saved.local_copy.status,
                    path: saved.local_copy.path,
                    reason: saved.local_copy.reason,
                },
                local_status: saved.local_status,
                local_path: saved.local_path,
                claim_status: saved.claim_status,
                claim_id: saved.claim_id,
                claim_error: saved.claim_error,
                next_step: SaveMemoryNextStepResponse {
                    tool: saved.next_step.tool,
                    ids: saved.next_step.ids,
                    source: saved.next_step.source,
                    reason: saved.next_step.reason,
                },
            }),
        )
            .into_response(),
        Err(err) => map_save_memory_error(err).into_response(),
    }
}

fn map_save_memory_error(err: anyhow::Error) -> impl IntoResponse {
    let msg = err.to_string();
    if err.is::<service::SaveMemoryValidationError>() {
        return error_response(StatusCode::BAD_REQUEST, "save_validation_failed", &msg);
    }
    if err.is::<service::LocalCopyError>() {
        return error_response(StatusCode::BAD_REQUEST, "save_local_copy_failed", &msg);
    }
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", &msg)
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, extract::State, http::StatusCode, response::IntoResponse, Json};
    use serde_json::Value;

    use crate::api::types::SaveMemoryRequest;
    use crate::db::test_support::ScopedTestDataDir;

    use super::{handle_save_memory, DbState};

    fn request_with_invalid_shape(
        memory_type: Option<&str>,
        scope: Option<&str>,
    ) -> SaveMemoryRequest {
        SaveMemoryRequest {
            text: "valid body".to_string(),
            title: Some("Invalid shape".to_string()),
            project: Some("proj".to_string()),
            session_id: None,
            host: None,
            topic_key: None,
            memory_type: memory_type.map(str::to_string),
            files: None,
            scope: scope.map(str::to_string),
            reference_time_epoch: None,
            created_at_epoch: None,
            branch: None,
            local_path: None,
            local_copy_enabled: Some(false),
            claim_enabled: None,
            claim_source: None,
        }
    }

    #[tokio::test]
    async fn save_memory_validation_errors_return_stable_bad_request() {
        let _dir = ScopedTestDataDir::new("api-save-validation");
        for req in [
            SaveMemoryRequest {
                text: "   ".to_string(),
                ..request_with_invalid_shape(Some("decision"), Some("project"))
            },
            request_with_invalid_shape(Some("decison"), Some("project")),
            request_with_invalid_shape(Some("decision"), Some("globla")),
        ] {
            let response = handle_save_memory(State(DbState), Json(req))
                .await
                .into_response();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let body = match to_bytes(response.into_body(), usize::MAX).await {
                Ok(body) => body,
                Err(err) => panic!("response body should read: {err}"),
            };
            let payload: Value = match serde_json::from_slice(&body) {
                Ok(payload) => payload,
                Err(err) => panic!("save response should be valid json: {err}"),
            };
            assert_eq!(payload["error"]["code"], "save_validation_failed");
        }
    }
}
