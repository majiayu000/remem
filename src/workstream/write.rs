use anyhow::Result;
use rusqlite::{params, Connection};

use super::{matcher::find_matching_workstream, ParsedWorkStream, WorkStreamStatus};

pub fn upsert_workstream(
    conn: &Connection,
    project: &str,
    memory_session_id: &str,
    parsed: &ParsedWorkStream,
) -> Result<i64> {
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

    let workstream_id = if let Some(existing) = find_matching_workstream(conn, project, title)? {
        conn.execute(
            "UPDATE workstreams
             SET title = ?1, status = ?2, progress = ?3, next_action = ?4, blockers = ?5,
                 updated_at_epoch = ?6, completed_at_epoch = COALESCE(?7, completed_at_epoch)
             WHERE id = ?8",
            params![
                title,
                status.as_str(),
                parsed.progress,
                parsed.next_action,
                parsed.blockers,
                now,
                completed_at,
                existing.id,
            ],
        )?;
        existing.id
    } else {
        conn.execute(
            "INSERT INTO workstreams
             (project, title, status, progress, next_action, blockers,
              created_at_epoch, updated_at_epoch, completed_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
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
        conn.last_insert_rowid()
    };

    conn.execute(
        "INSERT OR IGNORE INTO workstream_sessions (workstream_id, memory_session_id, linked_at_epoch)
         VALUES (?1, ?2, ?3)",
        params![workstream_id, memory_session_id, now],
    )?;

    Ok(workstream_id)
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
