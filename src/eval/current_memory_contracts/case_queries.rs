use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::fixture::PROJECT;

pub(super) fn current_state_result(
    conn: &Connection,
    state_key: &str,
    as_of_epoch: Option<i64>,
) -> Result<crate::memory::current_state::CurrentStateResult> {
    crate::memory::current_state::current_state(
        conn,
        &crate::memory::current_state::CurrentStateRequest {
            state_key: state_key.to_string(),
            project: Some(PROJECT.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch,
            include_history: true,
            ..Default::default()
        },
    )
}

pub(super) fn fact_objects(
    result: &crate::memory::current_state::CurrentStateResult,
) -> Vec<String> {
    result
        .facts
        .iter()
        .map(|fact| fact.object.clone())
        .collect()
}

pub(super) fn context_item_count(conn: &Connection, session_id: &str, status: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*)
         FROM context_injection_items
         WHERE session_id = ?1
           AND status = ?2",
        params![session_id, status],
        |row| row.get(0),
    )
    .context("count context injection items")
}

pub(super) fn context_item_id(
    conn: &Connection,
    session_id: &str,
    status: &str,
    memory_id: i64,
) -> Result<i64> {
    conn.query_row(
        "SELECT id
         FROM context_injection_items
         WHERE session_id = ?1
           AND status = ?2
           AND memory_id = ?3
         ORDER BY id DESC
         LIMIT 1",
        params![session_id, status, memory_id],
        |row| row.get(0),
    )
    .optional()?
    .with_context(|| {
        format!(
            "load context injection item for session={session_id} status={status} memory_id={memory_id}"
        )
    })
}

pub(super) fn context_item_drop_reason_for_memory(
    conn: &Connection,
    session_id: &str,
    memory_id: i64,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT drop_reason
         FROM context_injection_items
         WHERE session_id = ?1
           AND status = 'dropped'
           AND memory_id = ?2
         ORDER BY id DESC
         LIMIT 1",
        params![session_id, memory_id],
        |row| row.get(0),
    )
    .optional()
    .context("load dropped context item reason")
}

pub(super) fn context_abstention_row(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<(Option<i64>, Option<String>)>> {
    conn.query_row(
        "SELECT memory_id, drop_reason
         FROM context_injection_items
         WHERE session_id = ?1
           AND status = 'abstained'
         ORDER BY id DESC
         LIMIT 1",
        [session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .context("load abstained context item")
}

pub(super) fn citation_event_status(
    conn: &Connection,
    message_hash: &str,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT status
         FROM memory_citation_events
         WHERE message_hash = ?1
         LIMIT 1",
        [message_hash],
        |row| row.get(0),
    )
    .optional()
    .context("load memory citation event status")
}

pub(super) fn usage_event_count(
    conn: &Connection,
    message_hash: &str,
    memory_id: i64,
    context_injection_item_id: i64,
) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*)
         FROM memory_usage_events
         WHERE message_hash = ?1
           AND memory_id = ?2
           AND context_injection_item_id = ?3",
        params![message_hash, memory_id, context_injection_item_id],
        |row| row.get(0),
    )
    .context("count linked memory usage events")
}
