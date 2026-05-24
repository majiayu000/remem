use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde_json::Value;

use super::{ProcedurePromotionPolicy, ProcedureTrace};

pub(super) fn load_verified_procedure_traces(
    conn: &Connection,
    task: &crate::db::ExtractionTask,
    policy: &ProcedurePromotionPolicy,
    now_epoch: i64,
) -> Result<Vec<ProcedureTrace>> {
    let Some(session_row_id) = task.session_row_id else {
        return Ok(Vec::new());
    };
    let Some(high_watermark) = task.high_watermark_event_id else {
        return Ok(Vec::new());
    };
    let cursor = task.cursor_event_id.unwrap_or(0);
    if high_watermark <= cursor {
        return Ok(Vec::new());
    }

    let earliest = now_epoch.saturating_sub(policy.max_verification_age_secs);
    let new_traces =
        load_current_window_traces(conn, task, session_row_id, cursor, high_watermark, earliest)?;
    if new_traces.is_empty() {
        return Ok(Vec::new());
    }
    persist_verifications(conn, task, session_row_id, &new_traces, now_epoch)?;
    load_trace_history(
        conn,
        task,
        session_row_id,
        high_watermark,
        earliest,
        &new_traces,
    )
}

fn load_current_window_traces(
    conn: &Connection,
    task: &crate::db::ExtractionTask,
    session_row_id: i64,
    cursor: i64,
    high_watermark: i64,
    earliest: i64,
) -> Result<Vec<ProcedureTrace>> {
    let mut stmt = conn.prepare(
        "SELECT e.id,
                p.project_path,
                COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content,
                e.created_at_epoch
         FROM captured_events e
         JOIN projects p ON p.id = e.project_id
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.host_id = ?1
           AND e.project_id = ?2
           AND e.session_row_id = ?3
           AND e.id > ?4
           AND e.id <= ?5
           AND e.tool_name = 'Bash'
           AND e.created_at_epoch >= ?6
         ORDER BY e.created_at_epoch ASC, e.id ASC",
    )?;
    let rows = stmt.query_map(
        params![
            task.host_id,
            task.project_id,
            session_row_id,
            cursor,
            high_watermark,
            earliest
        ],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        },
    )?;

    let mut traces = Vec::new();
    for row in rows {
        let (event_id, project, content, created_at_epoch) = row?;
        if let Some(trace) = parse_procedure_trace(event_id, project, &content, created_at_epoch) {
            traces.push(trace);
        }
    }
    Ok(traces)
}

fn persist_verifications(
    conn: &Connection,
    task: &crate::db::ExtractionTask,
    session_row_id: i64,
    traces: &[ProcedureTrace],
    now_epoch: i64,
) -> Result<()> {
    for trace in traces {
        let source_event_id = trace
            .source_event_id
            .context("procedure trace missing source event id")?;
        let files_json = serde_json::to_string(&trace.files_touched)?;
        conn.execute(
            "INSERT INTO procedure_verifications
             (host_id, project_id, session_row_id, branch, workflow_key, command,
              files_touched, source_event_id, verified_at_epoch, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
             ON CONFLICT(host_id, project_id, session_row_id, source_event_id)
             DO UPDATE SET
                 branch = excluded.branch,
                 workflow_key = excluded.workflow_key,
                 command = excluded.command,
                 files_touched = excluded.files_touched,
                 verified_at_epoch = excluded.verified_at_epoch,
                 updated_at_epoch = excluded.updated_at_epoch",
            params![
                task.host_id,
                task.project_id,
                session_row_id,
                trace.branch.as_deref(),
                trace.workflow_key.as_str(),
                trace.command.as_str(),
                files_json,
                source_event_id,
                trace.verified_at_epoch,
                now_epoch
            ],
        )?;
    }
    Ok(())
}

fn load_trace_history(
    conn: &Connection,
    task: &crate::db::ExtractionTask,
    session_row_id: i64,
    high_watermark: i64,
    earliest: i64,
    new_traces: &[ProcedureTrace],
) -> Result<Vec<ProcedureTrace>> {
    let mut groups: BTreeMap<(Option<String>, String, String), ()> = BTreeMap::new();
    for trace in new_traces {
        groups.insert(
            (
                trace.branch.clone(),
                trace.workflow_key.clone(),
                trace.command.clone(),
            ),
            (),
        );
    }

    let mut traces = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT p.project_path,
                v.branch,
                v.workflow_key,
                v.command,
                v.files_touched,
                v.source_event_id,
                v.verified_at_epoch
         FROM procedure_verifications v
         JOIN projects p ON p.id = v.project_id
         WHERE v.host_id = ?1
           AND v.project_id = ?2
           AND v.session_row_id = ?3
           AND ((v.branch IS NULL AND ?4 IS NULL) OR v.branch = ?4)
           AND v.workflow_key = ?5
           AND v.command = ?6
           AND v.source_event_id <= ?7
           AND v.verified_at_epoch >= ?8
         ORDER BY v.verified_at_epoch ASC, v.source_event_id ASC",
    )?;
    for (branch, workflow_key, command) in groups.into_keys() {
        let rows = stmt.query_map(
            params![
                task.host_id,
                task.project_id,
                session_row_id,
                branch.as_deref(),
                workflow_key.as_str(),
                command.as_str(),
                high_watermark,
                earliest
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )?;
        for row in rows {
            let (
                project,
                branch,
                workflow_key,
                command,
                files_touched_json,
                source_event_id,
                verified_at_epoch,
            ) = row?;
            let files_touched = serde_json::from_str::<Vec<String>>(&files_touched_json)
                .with_context(|| {
                    format!("procedure verification {source_event_id} has malformed files_touched")
                })?;
            traces.push(ProcedureTrace {
                project,
                branch,
                workflow_key,
                command,
                files_touched,
                succeeded: true,
                verified_at_epoch,
                source_event_id: Some(source_event_id),
            });
        }
    }
    Ok(traces)
}

fn parse_procedure_trace(
    event_id: i64,
    project: String,
    content: &str,
    verified_at_epoch: i64,
) -> Option<ProcedureTrace> {
    let value: Value = serde_json::from_str(content).ok()?;
    if value.get("event_type")?.as_str()? != "bash" {
        return None;
    }
    if value.get("exit_code")?.as_i64()? != 0 {
        return None;
    }
    let command = value
        .get("tool_input")?
        .get("command")?
        .as_str()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if command.is_empty() {
        return None;
    }
    Some(ProcedureTrace {
        project,
        branch: parse_event_branch(&value),
        workflow_key: workflow_key_for_command(&command),
        command,
        files_touched: parse_event_files(&value),
        succeeded: true,
        verified_at_epoch,
        source_event_id: Some(event_id),
    })
}

fn parse_event_branch(value: &Value) -> Option<String> {
    value
        .get("git_branch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
}

fn parse_event_files(value: &Value) -> Vec<String> {
    let Some(files) = value.get("files") else {
        return Vec::new();
    };
    let mut parsed = match files {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        Value::String(raw) => match serde_json::from_str::<Vec<String>>(raw) {
            Ok(files) => files,
            Err(error) => {
                crate::log::warn(
                    "procedure",
                    &format!("ignored malformed procedure event files JSON: {error}"),
                );
                Vec::new()
            }
        },
        _ => Vec::new(),
    };
    parsed.sort();
    parsed.dedup();
    parsed
}

fn workflow_key_for_command(command: &str) -> String {
    crate::memory::slugify_for_topic(command, 64)
}
