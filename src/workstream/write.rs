use anyhow::Result;
use rusqlite::{params, Connection};

use super::{
    identity::{ensure_workstream_alias, workstream_identity_key, MATCH_REASON_INSERT},
    matcher::find_workstream_for_upsert,
    ParsedWorkStream, WorkStreamStatus, WorkStreamUpsertResult,
};

pub fn upsert_workstream(
    conn: &Connection,
    project: &str,
    memory_session_id: &str,
    parsed: &ParsedWorkStream,
) -> Result<i64> {
    Ok(upsert_workstream_with_match(conn, project, memory_session_id, parsed)?.id)
}

pub fn upsert_workstream_with_match(
    conn: &Connection,
    project: &str,
    memory_session_id: &str,
    parsed: &ParsedWorkStream,
) -> Result<WorkStreamUpsertResult> {
    let Some(title) = parsed.title.as_deref() else {
        anyhow::bail!("workstream title is required");
    };

    let now = chrono::Utc::now().timestamp();
    let status = if parsed.is_completed {
        WorkStreamStatus::Completed
    } else {
        WorkStreamStatus::Active
    };
    let completed_at = if parsed.is_completed { Some(now) } else { None };

    let (workstream_id, match_reason, previous_title) = if let Some(existing) =
        find_workstream_for_upsert(conn, project, memory_session_id, title)?
    {
        conn.execute(
            "UPDATE workstreams
             SET title = ?1, status = ?2, progress = ?3, next_action = ?4, blockers = ?5,
                 updated_at_epoch = ?6, completed_at_epoch = COALESCE(?7, completed_at_epoch),
                 source_project = COALESCE(source_project, ?9),
                 target_project = COALESCE(target_project, ?9),
                 owner_scope = COALESCE(owner_scope, 'repo'),
                 owner_key = COALESCE(owner_key, ?9),
                 context_class = COALESCE(context_class, 'startup_core')
             WHERE id = ?8",
            params![
                title,
                status.as_str(),
                parsed.progress,
                parsed.next_action,
                parsed.blockers,
                now,
                completed_at,
                existing.workstream.id,
                project,
            ],
        )?;
        (
            existing.workstream.id,
            existing.reason,
            Some(existing.workstream.title),
        )
    } else {
        conn.execute(
            "INSERT INTO workstreams
             (project, title, status, progress, next_action, blockers,
              created_at_epoch, updated_at_epoch, completed_at_epoch,
              source_project, target_project, owner_scope, owner_key, context_class)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?1, ?1, 'repo', ?1, 'startup_core')",
            params![
                project,
                title,
                status.as_str(),
                parsed.progress,
                parsed.next_action,
                parsed.blockers,
                now,
                completed_at,
            ],
        )?;
        let inserted_id = conn.last_insert_rowid();
        let identity_key = workstream_identity_key(project, memory_session_id, now, inserted_id);
        conn.execute(
            "UPDATE workstreams SET identity_key = ?1 WHERE id = ?2",
            params![identity_key, inserted_id],
        )?;
        (inserted_id, MATCH_REASON_INSERT, None)
    };

    if let Some(previous_title) = previous_title.as_deref().filter(|value| *value != title) {
        ensure_workstream_alias(
            conn,
            workstream_id,
            previous_title,
            "previous_title",
            None,
            None,
            now,
        )?;
    }
    ensure_workstream_alias(
        conn,
        workstream_id,
        title,
        "summary",
        Some(memory_session_id),
        None,
        now,
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO workstream_sessions (workstream_id, memory_session_id, linked_at_epoch)
         VALUES (?1, ?2, ?3)",
        params![workstream_id, memory_session_id, now],
    )?;

    Ok(WorkStreamUpsertResult {
        id: workstream_id,
        match_reason,
    })
}

pub fn update_workstream_manual(
    conn: &Connection,
    id: i64,
    status: Option<&str>,
    next_action: Option<&str>,
    blockers: Option<&str>,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let mut sets = vec!["updated_at_epoch = ?1".to_string()];
    let mut param_idx = 2u32;

    let status_val = status.map(WorkStreamStatus::from_db);
    if status_val.is_some() {
        sets.push(format!("status = ?{}", param_idx));
        param_idx += 1;
    }
    if next_action.is_some() {
        sets.push(format!("next_action = ?{}", param_idx));
        param_idx += 1;
    }
    if blockers.is_some() {
        sets.push(format!("blockers = ?{}", param_idx));
        param_idx += 1;
    }
    if status_val == Some(WorkStreamStatus::Completed) {
        sets.push(format!("completed_at_epoch = ?{}", param_idx));
        param_idx += 1;
    }

    let sql = format!(
        "UPDATE workstreams SET {} WHERE id = ?{}",
        sets.join(", "),
        param_idx
    );
    let completed_at = status_val
        .filter(|value| *value == WorkStreamStatus::Completed)
        .map(|_| now);

    let mut dynamic_params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
    if let Some(value) = &status_val {
        dynamic_params.push(Box::new(value.as_str().to_string()));
    }
    if let Some(value) = next_action {
        dynamic_params.push(Box::new(value.to_string()));
    }
    if let Some(value) = blockers {
        dynamic_params.push(Box::new(value.to_string()));
    }
    if let Some(value) = completed_at {
        dynamic_params.push(Box::new(value));
    }
    dynamic_params.push(Box::new(id));

    let refs = crate::db::to_sql_refs(&dynamic_params);
    let affected = conn.execute(&sql, refs.as_slice())?;
    Ok(affected > 0)
}
