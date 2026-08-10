use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::{db, memory};

use super::types::{ErrorDetail, ErrorResponse, MemoryItem};

pub(super) fn memory_to_item_with_conn(
    conn: &rusqlite::Connection,
    memory: &memory::Memory,
) -> anyhow::Result<MemoryItem> {
    let now_epoch = chrono::Utc::now().timestamp();
    let staleness = memory::memory_staleness_label_with_conn(conn, memory, now_epoch)?;
    let mut item = memory_to_item_with_staleness(memory, staleness);
    annotate_visibility(conn, memory, now_epoch, &mut item)?;
    Ok(item)
}

pub(super) fn memories_to_items_with_conn(
    conn: &rusqlite::Connection,
    memories: &[memory::Memory],
) -> anyhow::Result<Vec<MemoryItem>> {
    let now_epoch = chrono::Utc::now().timestamp();
    let staleness_labels =
        memory::staleness::memory_staleness_labels_for_memories(conn, memories, now_epoch)?;
    memories
        .iter()
        .map(|memory| -> anyhow::Result<MemoryItem> {
            let staleness = staleness_labels
                .get(&memory.id)
                .cloned()
                .unwrap_or_else(|| memory::memory_staleness_label(memory, now_epoch));
            let mut item = memory_to_item_with_staleness(memory, staleness);
            annotate_visibility(conn, memory, now_epoch, &mut item)?;
            Ok(item)
        })
        .collect()
}

fn annotate_visibility(
    conn: &rusqlite::Connection,
    memory: &memory::Memory,
    as_of_epoch: i64,
    item: &mut MemoryItem,
) -> anyhow::Result<()> {
    let visibility = crate::truth::classify_memory(conn, memory.id, as_of_epoch)?;
    item.classification = visibility.classification;
    item.current_context_eligible = visibility.current_context_eligible;
    item.classification_reason = visibility.reason.as_str().to_string();
    Ok(())
}

fn memory_to_item_with_staleness(
    memory: &memory::Memory,
    staleness: memory::MemoryStalenessLabel,
) -> MemoryItem {
    MemoryItem {
        id: memory.id,
        version: None,
        title: memory.title.clone(),
        content: memory.text.clone(),
        memory_type: memory.memory_type.clone(),
        project: memory.project.clone(),
        scope: memory.scope.clone(),
        status: memory.status.clone(),
        classification: crate::truth::MemoryVisibilityClass::LegacyUnverified,
        current_context_eligible: false,
        classification_reason: "legacy_unverified_unclassified".to_string(),
        staleness,
        topic_key: memory.topic_key.clone(),
        branch: memory.branch.clone(),
        created_at_epoch: memory.created_at_epoch,
        updated_at_epoch: memory.updated_at_epoch,
    }
}

pub(super) fn error_response(status: StatusCode, code: &str, message: &str) -> impl IntoResponse {
    (
        status,
        Json(ErrorResponse {
            error: ErrorDetail {
                code: code.to_string(),
                message: message.to_string(),
            },
        }),
    )
}

pub(super) fn staleness_error_response(err: &anyhow::Error) -> impl IntoResponse {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "staleness_source_anchor_failed",
        &err.to_string(),
    )
}

#[allow(clippy::result_large_err)]
pub(super) fn open_request_db() -> Result<rusqlite::Connection, Response> {
    db::open_db().map_err(|err| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "db_open_failed",
            &err.to_string(),
        )
        .into_response()
    })
}
