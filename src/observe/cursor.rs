//! Cursor `remem observe` capture path (GH-823 B-007, B-011, B-014..B-016).
//!
//! The strict parser in `crate::cursor_hook::input` runs before any database
//! open or side effect; only its sanitized canonical event reaches capture,
//! spill, or persistence. `tool_use_id` is the canonical per-call event/upsert
//! identity; under B-016 generic ownership the generic post-tool event is the
//! sole writer and MCP-specific events are rejected in the parser.

use anyhow::Result;
use rusqlite::OptionalExtension;

use crate::cursor_hook::input::{CursorToolEvent, CursorToolOutcome};
use crate::cursor_hook::{CURSOR_HOST, CURSOR_TOOL_FAILURE_EVENT_TYPE};
use crate::db;

use super::spill::{
    replay_spilled_capture_events, spill_capture_event_with_git_evidence,
    SPILL_REASON_CAPTURE_PERSISTENCE_FAILED, SPILL_REASON_DB_OPEN_FAILED,
};

pub async fn observe_cursor() -> Result<()> {
    let bytes = crate::cursor_hook::input::read_bounded_hook_stdin(&mut std::io::stdin().lock())?;
    observe_cursor_bytes(&bytes).await
}

pub async fn observe_cursor_bytes(bytes: &[u8]) -> Result<()> {
    let event = crate::cursor_hook::input::parse_observe_event(bytes)?;
    record_cursor_tool_event(&event)
}

fn record_cursor_tool_event(event: &CursorToolEvent) -> Result<()> {
    let parsed = to_parsed_hook_event(event);
    let summary = cursor_event_summary(event);
    let event_id = cursor_tool_event_id(&event.tool_use_id);
    let conn = match db::open_db_for_hook() {
        Ok(conn) => conn,
        Err(error) => {
            let path = spill_capture_event_with_git_evidence(
                CURSOR_HOST,
                &event_id,
                &parsed,
                &summary,
                &[],
                SPILL_REASON_DB_OPEN_FAILED,
                &error,
            )?;
            crate::log::error(
                "observe",
                &format!(
                    "database open failed; spilled cursor capture event to {}: {}",
                    path.display(),
                    error
                ),
            );
            return Err(error);
        }
    };
    replay_spilled_capture_events(&conn)?;

    let existing = existing_cursor_event(&conn, &event.session_id, &event_id)?;
    match (existing.as_ref(), event.outcome.is_failure()) {
        (Some((_, existing_type)), false) if existing_type == CURSOR_TOOL_FAILURE_EVENT_TYPE => {
            crate::log::info(
                "observe",
                &format!(
                    "cursor tool failure remains authoritative over late success session={} event={}",
                    event.session_id, event_id
                ),
            );
            Ok(())
        }
        (Some((captured_event_id, _)), false) => {
            // Replay of a call maps to itself: the canonical per-call key is
            // already captured, and a late success never downgrades an
            // already-recorded failure (failure precedence).
            crate::memory::insert_event_for_capture(
                &conn,
                *captured_event_id,
                &event.session_id,
                &event.workspace_root,
                &summary.event_type,
                &summary.summary,
                summary.detail.as_deref(),
                None,
                None,
            )?;
            crate::log::info(
                "observe",
                &format!(
                    "cursor tool event already captured; idempotent replay session={} event={}",
                    event.session_id, event_id
                ),
            );
            Ok(())
        }
        (Some((captured_event_id, existing_type)), true)
            if existing_type == CURSOR_TOOL_FAILURE_EVENT_TYPE =>
        {
            crate::memory::replace_event_for_capture(
                &conn,
                *captured_event_id,
                &event.session_id,
                &event.workspace_root,
                &summary.event_type,
                &summary.summary,
                summary.detail.as_deref(),
                None,
                None,
            )?;
            crate::log::info(
                "observe",
                &format!(
                    "cursor tool failure already captured; idempotent replay session={} event={}",
                    event.session_id, event_id
                ),
            );
            Ok(())
        }
        (Some((captured_event_id, _)), true) => {
            promote_cursor_failure(&conn, event, &event_id, *captured_event_id, &summary)
        }
        (None, _) => {
            if let Err(error) = super::hook::record_observed_event_with_id(
                &conn,
                CURSOR_HOST,
                &event_id,
                &parsed,
                &summary,
                &[],
            ) {
                let path = spill_capture_event_with_git_evidence(
                    CURSOR_HOST,
                    &event_id,
                    &parsed,
                    &summary,
                    &[],
                    SPILL_REASON_CAPTURE_PERSISTENCE_FAILED,
                    &error,
                )?;
                crate::log::error(
                    "observe",
                    &format!(
                        "cursor capture persistence failed; spilled capture event to {}: {}",
                        path.display(),
                        error
                    ),
                );
                return Err(error);
            }
            Ok(())
        }
    }
}

/// Failure precedence for dual delivery on one `tool_use_id`: the captured
/// row is promoted to the existing `cursor_tool_failure` text discriminator
/// exactly once, and the linked legacy projection follows that authoritative
/// outcome in the same savepoint.
fn promote_cursor_failure(
    conn: &rusqlite::Connection,
    event: &CursorToolEvent,
    event_id: &str,
    captured_event_id: i64,
    summary: &crate::adapter::EventSummary,
) -> Result<()> {
    super::hook::with_observed_projection_savepoint(conn, || {
        let updated = conn.execute(
            "UPDATE captured_events
             SET event_type = ?1
             WHERE id = ?2 AND session_id = ?3 AND event_id = ?4
               AND host_id = (SELECT id FROM hosts WHERE name = ?5)",
            rusqlite::params![
                CURSOR_TOOL_FAILURE_EVENT_TYPE,
                captured_event_id,
                event.session_id,
                event_id,
                CURSOR_HOST
            ],
        )?;
        anyhow::ensure!(
            updated == 1,
            "cursor canonical failure promotion target drifted"
        );
        crate::memory::replace_event_for_capture(
            conn,
            captured_event_id,
            &event.session_id,
            &event.workspace_root,
            &summary.event_type,
            &summary.summary,
            summary.detail.as_deref(),
            None,
            None,
        )?;
        Ok(())
    })?;
    crate::log::info(
        "observe",
        &format!(
            "cursor tool failure took precedence over captured success session={} event={}",
            event.session_id, event_id
        ),
    );
    Ok(())
}

fn existing_cursor_event(
    conn: &rusqlite::Connection,
    session_id: &str,
    event_id: &str,
) -> Result<Option<(i64, String)>> {
    Ok(conn
        .query_row(
            "SELECT ce.id, ce.event_type FROM captured_events ce
             JOIN hosts h ON h.id = ce.host_id
             WHERE h.name = ?1 AND ce.session_id = ?2 AND ce.event_id = ?3",
            rusqlite::params![CURSOR_HOST, session_id, event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?)
}

/// Canonical per-call event id derived from the human-approved `tool_use_id`
/// (SP823-T2 item 5): two same-tool calls in one generation keep distinct
/// keys, replay of a call maps to itself.
pub(super) fn cursor_tool_event_id(tool_use_id: &str) -> String {
    format!("cursor-tool:{tool_use_id}")
}

fn to_parsed_hook_event(event: &CursorToolEvent) -> crate::adapter::ParsedHookEvent {
    let tool_response = match &event.outcome {
        CursorToolOutcome::Success { tool_output } => {
            serde_json::Value::String(tool_output.clone())
        }
        CursorToolOutcome::Failure {
            error_message,
            failure_type,
            duration,
            is_interrupt,
        } => serde_json::json!({
            "error_message": error_message,
            "failure_type": failure_type,
            "duration": duration,
            "is_interrupt": is_interrupt,
        }),
    };
    crate::adapter::ParsedHookEvent {
        session_id: event.session_id.clone(),
        cwd: Some(event.workspace_root.clone()),
        project: event.workspace_root.clone(),
        reference_time_epoch: None,
        tool_name: event.tool_name.clone(),
        tool_input: Some(serde_json::Value::Object(event.tool_input.clone())),
        tool_response: Some(tool_response),
    }
}

fn cursor_event_summary(event: &CursorToolEvent) -> crate::adapter::EventSummary {
    match &event.outcome {
        CursorToolOutcome::Success { .. } => crate::adapter::EventSummary {
            event_type: "tool_result".to_string(),
            summary: format!("Cursor {} completed", event.tool_name),
            detail: None,
            files_json: None,
            exit_code: None,
        },
        CursorToolOutcome::Failure {
            failure_type,
            duration,
            is_interrupt,
            ..
        } => crate::adapter::EventSummary {
            event_type: CURSOR_TOOL_FAILURE_EVENT_TYPE.to_string(),
            summary: format!("Cursor {} failed", event.tool_name),
            detail: Some(format!(
                "failure_type={failure_type}; duration={duration}; is_interrupt={is_interrupt}"
            )),
            files_json: None,
            exit_code: None,
        },
    }
}
