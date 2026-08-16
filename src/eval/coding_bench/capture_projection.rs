use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::types::{CodingBenchTask, RawHistoryEvent};

pub(super) const PROJECTION_SCHEMA: &str = "remem_e2e_capture_projection_v1";
pub(super) const FIXED_PROJECT_PATH: &str = "/workspace/remem-e2e/project";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct CaptureProjection {
    schema: &'static str,
    events: Vec<ProjectedEvent>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectedEvent {
    pub source_ordinal: usize,
    pub event_id: String,
    pub timestamp_epoch: i64,
    pub role: String,
    pub sanitized_content: String,
    pub tool_name: Option<String>,
    pub sanitized_tool_input: Option<String>,
    pub sanitized_tool_output: Option<String>,
    pub host_boundary: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CaptureCallContent<'a> {
    host_boundary: &'a str,
    sanitized_content: &'a str,
    sanitized_tool_input: Option<&'a str>,
    sanitized_tool_output: Option<&'a str>,
    source_ordinal: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CapturePlan {
    pub projection: CaptureProjection,
    pub projection_sha256: String,
    pub calls: Vec<CaptureCall>,
    pub call_plan_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaptureCall {
    pub source_ordinal: usize,
    pub event_id: String,
    pub timestamp_epoch: i64,
    pub event_type: &'static str,
    pub role: String,
    pub tool_name: Option<String>,
    pub canonical_content: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(super) struct CaptureWriteTrace {
    pub captured_count: usize,
    pub first_event_row_id: i64,
    pub last_event_row_id: i64,
}

pub(super) fn build_capture_plan(task: &CodingBenchTask) -> Result<CapturePlan> {
    let raw_events = task
        .history_episodes
        .iter()
        .flat_map(|episode| episode.raw_events.iter())
        .collect::<Vec<_>>();
    if raw_events.is_empty() {
        bail!("remem_e2e task {} has no raw history events", task.id);
    }

    let mut seen_ids = BTreeSet::new();
    let mut previous_timestamp = None;
    let mut events = Vec::with_capacity(raw_events.len());
    for (source_ordinal, event) in raw_events.into_iter().enumerate() {
        validate_raw_event(task, event, source_ordinal)?;
        if !seen_ids.insert(event.event_id.as_str()) {
            bail!(
                "remem_e2e task {} repeats raw event id {}",
                task.id,
                event.event_id
            );
        }
        if previous_timestamp.is_some_and(|previous| event.timestamp_epoch < previous) {
            bail!(
                "remem_e2e task {} raw event {} timestamp decreases at source ordinal {}",
                task.id,
                event.event_id,
                source_ordinal
            );
        }
        previous_timestamp = Some(event.timestamp_epoch);
        events.push(ProjectedEvent {
            source_ordinal,
            event_id: event.event_id.clone(),
            timestamp_epoch: event.timestamp_epoch,
            role: event.role.clone(),
            sanitized_content: event.sanitized_content.clone(),
            tool_name: event.tool_name.clone(),
            sanitized_tool_input: event.sanitized_tool_input.clone(),
            sanitized_tool_output: event.sanitized_tool_output.clone(),
            host_boundary: event.host_boundary.clone(),
        });
    }

    let projection = CaptureProjection {
        schema: PROJECTION_SCHEMA,
        events,
    };
    let projection_bytes =
        crate::api::mutation::canonical_json_bytes(&serde_json::to_value(&projection)?)?;
    let calls = projection
        .events
        .iter()
        .map(build_capture_call)
        .collect::<Result<Vec<_>>>()?;
    let call_values = calls
        .iter()
        .map(|call| {
            serde_json::json!({
                "canonical_content": call.canonical_content,
                "event_id": call.event_id,
                "event_type": call.event_type,
                "role": call.role,
                "source_ordinal": call.source_ordinal,
                "timestamp_epoch": call.timestamp_epoch,
                "tool_name": call.tool_name,
            })
        })
        .collect::<Vec<_>>();
    let call_plan_bytes = crate::api::mutation::canonical_json_bytes(&Value::Array(call_values))?;

    Ok(CapturePlan {
        projection,
        projection_sha256: format!("{:x}", Sha256::digest(&projection_bytes)),
        calls,
        call_plan_sha256: format!("{:x}", Sha256::digest(&call_plan_bytes)),
    })
}

pub(super) fn write_capture_plan(
    conn: &Connection,
    plan: &CapturePlan,
    session_id: &str,
) -> Result<CaptureWriteTrace> {
    validate_session_id(session_id)?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .context("begin remem_e2e capture transaction")?;
    let result = write_capture_plan_in_transaction(conn, plan, session_id);
    match result {
        Ok(trace) => {
            conn.execute_batch("COMMIT")
                .context("commit remem_e2e capture transaction")?;
            Ok(trace)
        }
        Err(error) => match conn.execute_batch("ROLLBACK") {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "remem_e2e capture rollback also failed: {rollback_error}"
            ))),
        },
    }
}

fn write_capture_plan_in_transaction(
    conn: &Connection,
    plan: &CapturePlan,
    session_id: &str,
) -> Result<CaptureWriteTrace> {
    let existing: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM captured_events e
         JOIN hosts h ON h.id = e.host_id
         WHERE h.name = 'codex-cli' AND e.session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    if existing != 0 {
        bail!("remem_e2e capture session already contains events");
    }

    let mut row_ids = Vec::with_capacity(plan.calls.len());
    for call in &plan.calls {
        let redacted = crate::db::capture::redact_capture_content(&call.canonical_content);
        if redacted.as_bytes() != call.canonical_content.as_bytes() {
            bail!(
                "remem_e2e event {} changed under the production capture redactor",
                call.event_id
            );
        }
        let outcome = crate::db::record_captured_event_with_id_and_reference_time(
            conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id,
                project: FIXED_PROJECT_PATH,
                cwd: Some(FIXED_PROJECT_PATH),
                event_type: call.event_type,
                role: Some(&call.role),
                tool_name: call.tool_name.as_deref(),
                content: &call.canonical_content,
                task_kind: Some(crate::db::ExtractionTaskKind::ObservationExtract),
            },
            Some(&call.event_id),
            Some(call.timestamp_epoch),
        )?;
        row_ids.push(outcome.event_row_id);
    }

    if row_ids.windows(2).any(|window| window[0] >= window[1]) {
        bail!("remem_e2e captured event row IDs do not increase with source ordinal");
    }
    verify_written_rows(conn, plan, session_id, &row_ids)?;
    let first_event_row_id = *row_ids.first().context("missing first captured row")?;
    let last_event_row_id = *row_ids.last().context("missing last captured row")?;
    Ok(CaptureWriteTrace {
        captured_count: row_ids.len(),
        first_event_row_id,
        last_event_row_id,
    })
}

fn verify_written_rows(
    conn: &Connection,
    plan: &CapturePlan,
    session_id: &str,
    row_ids: &[i64],
) -> Result<()> {
    let mut statement = conn.prepare(
        "SELECT e.id, e.event_id, e.event_type, e.role, e.tool_name,
                COALESCE(CASE WHEN b.content_encoding = 'plain'
                              THEN CAST(b.content_bytes AS TEXT) END, e.content_text, ''),
                e.content_hash, e.reference_time_epoch, p.project_path, s.session_id
         FROM captured_events e
         JOIN projects p ON p.id = e.project_id
         JOIN sessions s ON s.id = e.session_row_id
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         JOIN hosts h ON h.id = e.host_id
         WHERE h.name = 'codex-cli' AND e.session_id = ?1
         ORDER BY e.id ASC",
    )?;
    let rows = statement
        .query_map([session_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != plan.calls.len() || rows.len() != row_ids.len() {
        bail!("remem_e2e call-plan/captured-row cardinality mismatch");
    }
    for ((call, row_id), row) in plan.calls.iter().zip(row_ids).zip(rows) {
        let expected_hash = crate::db::content_identity_hash(call.canonical_content.as_bytes());
        if row.0 != *row_id
            || row.1 != call.event_id
            || row.2 != call.event_type
            || row.3.as_deref() != Some(call.role.as_str())
            || row.4 != call.tool_name
            || row.5 != call.canonical_content
            || row.6 != expected_hash
            || row.7 != Some(call.timestamp_epoch)
            || row.8 != FIXED_PROJECT_PATH
            || row.9 != session_id
        {
            bail!(
                "remem_e2e captured row differs from call plan at source ordinal {}",
                call.source_ordinal
            );
        }
    }
    Ok(())
}

fn build_capture_call(event: &ProjectedEvent) -> Result<CaptureCall> {
    let content = CaptureCallContent {
        host_boundary: &event.host_boundary,
        sanitized_content: &event.sanitized_content,
        sanitized_tool_input: event.sanitized_tool_input.as_deref(),
        sanitized_tool_output: event.sanitized_tool_output.as_deref(),
        source_ordinal: event.source_ordinal,
    };
    let canonical_content = String::from_utf8(crate::api::mutation::canonical_json_bytes(
        &serde_json::to_value(content)?,
    )?)?;
    let event_type = match event.host_boundary.as_str() {
        "user_message" => "user_prompt_submit",
        "assistant_message" => "assistant_message",
        "tool_call" => "tool_call",
        "tool_result" => "tool_result",
        _ => unreachable!("host boundary was validated before call construction"),
    };
    Ok(CaptureCall {
        source_ordinal: event.source_ordinal,
        event_id: event.event_id.clone(),
        timestamp_epoch: event.timestamp_epoch,
        event_type,
        role: event.role.clone(),
        tool_name: event.tool_name.clone(),
        canonical_content,
    })
}

fn validate_raw_event(
    task: &CodingBenchTask,
    event: &RawHistoryEvent,
    source_ordinal: usize,
) -> Result<()> {
    if !valid_opaque_id(&event.event_id, "evt-") {
        bail!(
            "task {} raw event {} at ordinal {} must use evt- plus 32 lowercase hex characters",
            task.id,
            event.event_id,
            source_ordinal
        );
    }
    if event.timestamp_epoch <= 0 {
        bail!("task {} raw event timestamp must be positive", task.id);
    }
    let nonempty_content = !event.sanitized_content.trim().is_empty();
    let nonempty_tool_name = event
        .tool_name
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let nonempty_input = event
        .sanitized_tool_input
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let nonempty_output = event
        .sanitized_tool_output
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let valid_shape = match event.host_boundary.as_str() {
        "user_message" => {
            event.role == "user"
                && nonempty_content
                && event.tool_name.is_none()
                && event.sanitized_tool_input.is_none()
                && event.sanitized_tool_output.is_none()
        }
        "assistant_message" => {
            event.role == "assistant"
                && nonempty_content
                && event.tool_name.is_none()
                && event.sanitized_tool_input.is_none()
                && event.sanitized_tool_output.is_none()
        }
        "tool_call" => {
            event.role == "assistant"
                && nonempty_tool_name
                && nonempty_input
                && event.sanitized_tool_output.is_none()
        }
        "tool_result" => {
            event.role == "tool"
                && nonempty_tool_name
                && nonempty_output
                && event.sanitized_tool_input.is_none()
        }
        _ => false,
    };
    if !valid_shape {
        bail!(
            "task {} raw event {} has an invalid role/tool/host_boundary combination",
            task.id,
            event.event_id
        );
    }
    Ok(())
}

pub(super) fn new_opaque_id(prefix: &str) -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("generate remem_e2e opaque identity: {error}"))?;
    let mut id = String::with_capacity(prefix.len() + 32);
    id.push_str(prefix);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut id, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(id)
}

fn validate_session_id(value: &str) -> Result<()> {
    if valid_opaque_id(value, "e2e-") {
        Ok(())
    } else {
        bail!("remem_e2e session ID must use e2e- plus 32 lowercase hex characters")
    }
}

fn valid_opaque_id(value: &str, prefix: &str) -> bool {
    value.len() == prefix.len() + 32
        && value.starts_with(prefix)
        && value[prefix.len()..]
            .chars()
            .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::coding_bench::fixture::load_fixture;

    #[test]
    fn registered_fixture_builds_ordered_capture_plans() -> Result<()> {
        let fixture = load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        for task in &fixture.tasks {
            let plan = build_capture_plan(task)?;
            assert!(!plan.calls.is_empty());
            assert_eq!(plan.calls.len(), plan.projection.events.len());
            assert!(plan
                .calls
                .iter()
                .enumerate()
                .all(|(index, call)| index == call.source_ordinal));
        }
        Ok(())
    }

    #[test]
    fn capture_batch_is_insert_only_ordered_and_atomic() -> Result<()> {
        let fixture = load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let plan = build_capture_plan(&fixture.tasks[0])?;
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let session_id = new_opaque_id("e2e-")?;
        let trace = write_capture_plan(&conn, &plan, &session_id)?;
        assert_eq!(trace.captured_count, plan.calls.len());
        assert!(trace.first_event_row_id <= trace.last_event_row_id);

        let error = write_capture_plan(&conn, &plan, &session_id)
            .expect_err("a second batch with the same session must fail closed");
        assert!(error.to_string().contains("already contains events"));
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM captured_events", [], |row| row.get(0))?;
        assert_eq!(count as usize, plan.calls.len());
        Ok(())
    }

    #[test]
    fn projection_rejects_decreasing_time_and_invalid_boundary_shape() -> Result<()> {
        let fixture = load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let mut task = fixture.tasks[0].clone();
        let episode = &mut task.history_episodes[0];
        let mut second = episode.raw_events[0].clone();
        second.event_id = "evt-ffffffffffffffffffffffffffffffff".to_string();
        second.timestamp_epoch -= 1;
        episode.raw_events.push(second);
        assert!(build_capture_plan(&task)
            .unwrap_err()
            .to_string()
            .contains("timestamp decreases"));

        let mut task = fixture.tasks[0].clone();
        task.history_episodes[0].raw_events[0].role = "assistant".to_string();
        assert!(build_capture_plan(&task)
            .unwrap_err()
            .to_string()
            .contains("invalid role/tool/host_boundary"));
        Ok(())
    }
}
