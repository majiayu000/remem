use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::types::{
    ActivityCount, RawSessionActivity, SessionActivityItem, SessionActivityStats, SessionTurn,
    TurnAction,
};

const MAX_VISIBLE_USER_CHARS: usize = 4_000;

pub fn list_activity_sessions(
    conn: &Connection,
    project: Option<&str>,
    before_epoch: Option<i64>,
    limit: i64,
) -> Result<Vec<RawSessionActivity>> {
    let mut stmt = conn.prepare(
        "SELECT r.source_root, r.project, r.session_id, COUNT(*),
                SUM(CASE WHEN r.role = 'user' THEN 1 ELSE 0 END),
                SUM(CASE WHEN r.role = 'assistant' THEN 1 ELSE 0 END),
                MIN(r.created_at_epoch), MAX(r.created_at_epoch),
                (SELECT COUNT(*) FROM session_turns t
                 WHERE t.source_root = r.source_root
                   AND t.project = r.project
                   AND t.session_id = r.session_id)
         FROM raw_messages r
         WHERE (?1 IS NULL OR r.project = ?1)
         GROUP BY r.source_root, r.project, r.session_id
         HAVING (?2 IS NULL OR MAX(r.created_at_epoch) < ?2)
         ORDER BY MAX(r.created_at_epoch) DESC, r.source_root, r.project, r.session_id
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![project, before_epoch, limit], |row| {
        Ok(RawSessionActivity {
            source_root: row.get(0)?,
            project: row.get(1)?,
            session_id: row.get(2)?,
            message_count: row.get(3)?,
            user_message_count: row.get(4)?,
            assistant_message_count: row.get(5)?,
            first_epoch: row.get(6)?,
            last_epoch: row.get(7)?,
            projected_turn_count: row.get(8)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub fn list_turns(
    conn: &Connection,
    project: Option<&str>,
    source_root: Option<&str>,
    session_id: Option<&str>,
    before_id: Option<i64>,
    limit: i64,
) -> Result<Vec<SessionActivityItem>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.source_root, t.project, t.session_id, t.turn_index,
                t.user_message_id, u.content, t.understanding_message_id,
                t.understanding, t.understanding_source, t.result_message_id,
                t.actions_summary, t.result_status, t.result_summary,
                t.started_at_epoch, t.ended_at_epoch, t.capture_health
         FROM session_turns t
         JOIN raw_messages u ON u.id = t.user_message_id
         WHERE (?1 IS NULL OR t.project = ?1)
           AND (?2 IS NULL OR t.source_root = ?2)
           AND (?3 IS NULL OR t.session_id = ?3)
           AND (?4 IS NULL OR t.id < ?4)
         ORDER BY t.id DESC LIMIT ?5",
    )?;
    let rows = stmt
        .query_map(
            params![project, source_root, session_id, before_id, limit],
            map_activity_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|mut item| {
            let turn_id = item.turn.id.context("stored turn is missing id")?;
            item.turn.actions = load_actions(conn, turn_id)?;
            Ok(item)
        })
        .collect()
}

pub fn get_turn(conn: &Connection, id: i64) -> Result<Option<SessionActivityItem>> {
    let mut item = conn
        .query_row(
            "SELECT t.id, t.source_root, t.project, t.session_id, t.turn_index,
                    t.user_message_id, u.content, t.understanding_message_id,
                    t.understanding, t.understanding_source, t.result_message_id,
                    t.actions_summary, t.result_status, t.result_summary,
                    t.started_at_epoch, t.ended_at_epoch, t.capture_health
             FROM session_turns t
             JOIN raw_messages u ON u.id = t.user_message_id
             WHERE t.id = ?1",
            [id],
            map_activity_row,
        )
        .optional()?;
    if let Some(item) = &mut item {
        item.turn.actions = load_actions(conn, id)?;
    }
    Ok(item)
}

pub fn activity_stats(
    conn: &Connection,
    project: Option<&str>,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
) -> Result<SessionActivityStats> {
    let filter = "(?1 IS NULL OR project = ?1)
                  AND (?2 IS NULL OR started_at_epoch >= ?2)
                  AND (?3 IS NULL OR started_at_epoch <= ?3)";
    let (sessions, turns): (i64, i64) = conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(turn_count), 0)
             FROM (
               SELECT source_root, project, session_id, COUNT(*) AS turn_count
               FROM session_turns WHERE {filter}
               GROUP BY source_root, project, session_id
             )"
        ),
        params![project, since_epoch, until_epoch],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let actions = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM session_turn_actions a
             JOIN session_turns t ON t.id = a.session_turn_id WHERE {}",
            filter
                .replace("project", "t.project")
                .replace("started_at_epoch", "t.started_at_epoch")
        ),
        params![project, since_epoch, until_epoch],
        |row| row.get(0),
    )?;

    Ok(SessionActivityStats {
        sessions,
        turns,
        actions,
        result_status: grouped_turn_counts(
            conn,
            "result_status",
            project,
            since_epoch,
            until_epoch,
            20,
        )?,
        capture_health: grouped_turn_counts(
            conn,
            "capture_health",
            project,
            since_epoch,
            until_epoch,
            20,
        )?,
        projects: grouped_turn_counts(conn, "project", project, since_epoch, until_epoch, 10)?,
        tools: grouped_action_counts(conn, project, since_epoch, until_epoch)?,
    })
}

fn map_activity_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionActivityItem> {
    Ok(SessionActivityItem {
        source_root: row.get(1)?,
        project: row.get(2)?,
        session_id: row.get(3)?,
        turn: SessionTurn {
            id: Some(row.get(0)?),
            turn_index: row.get(4)?,
            user_message_id: row.get(5)?,
            user_said: bounded_visible_text(row.get::<_, String>(6)?),
            understanding_message_id: row.get(7)?,
            understanding: row.get(8)?,
            understanding_source: row.get(9)?,
            result_message_id: row.get(10)?,
            actions_summary: row.get(11)?,
            result_status: row.get(12)?,
            result_summary: row.get(13)?,
            started_at_epoch: row.get(14)?,
            ended_at_epoch: row.get(15)?,
            capture_health: row.get(16)?,
            actions: Vec::new(),
        },
    })
}

fn bounded_visible_text(value: String) -> String {
    if value.chars().count() <= MAX_VISIBLE_USER_CHARS {
        return value;
    }
    let mut bounded = value
        .chars()
        .take(MAX_VISIBLE_USER_CHARS)
        .collect::<String>();
    bounded.push('…');
    bounded
}

fn load_actions(conn: &Connection, turn_id: i64) -> Result<Vec<TurnAction>> {
    let mut stmt = conn.prepare(
        "SELECT action_index, kind, tool_name, summary, event_row_id,
                files_json, outcome, created_at_epoch
         FROM session_turn_actions WHERE session_turn_id = ?1
         ORDER BY action_index",
    )?;
    let rows = stmt
        .query_map([turn_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(index, kind, tool_name, summary, event_row_id, files_json, outcome, created)| {
                let files = serde_json::from_str(&files_json)
                    .context("decode session turn action files_json")?;
                Ok(TurnAction {
                    index,
                    kind,
                    tool_name,
                    summary,
                    event_row_id,
                    files,
                    outcome,
                    created_at_epoch: created,
                })
            },
        )
        .collect()
}

fn grouped_turn_counts(
    conn: &Connection,
    column: &str,
    project: Option<&str>,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    limit: i64,
) -> Result<Vec<ActivityCount>> {
    let sql = format!(
        "SELECT {column}, COUNT(*) FROM session_turns
         WHERE (?1 IS NULL OR project = ?1)
           AND (?2 IS NULL OR started_at_epoch >= ?2)
           AND (?3 IS NULL OR started_at_epoch <= ?3)
         GROUP BY {column} ORDER BY COUNT(*) DESC, {column} LIMIT ?4"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project, since_epoch, until_epoch, limit], |row| {
        Ok(ActivityCount {
            key: row.get(0)?,
            count: row.get(1)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn grouped_action_counts(
    conn: &Connection,
    project: Option<&str>,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
) -> Result<Vec<ActivityCount>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(a.tool_name, a.kind), COUNT(*)
         FROM session_turn_actions a JOIN session_turns t ON t.id = a.session_turn_id
         WHERE (?1 IS NULL OR t.project = ?1)
           AND (?2 IS NULL OR t.started_at_epoch >= ?2)
           AND (?3 IS NULL OR t.started_at_epoch <= ?3)
         GROUP BY COALESCE(a.tool_name, a.kind)
         ORDER BY COUNT(*) DESC, COALESCE(a.tool_name, a.kind) LIMIT 10",
    )?;
    let rows = stmt.query_map(params![project, since_epoch, until_epoch], |row| {
        Ok(ActivityCount {
            key: row.get(0)?,
            count: row.get(1)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}
