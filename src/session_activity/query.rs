use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::types::{
    ActivityCount, RawSessionActivity, RawSessionActivityPage, SessionActivityItem,
    SessionActivityPage, SessionActivityStats, SessionTurn, TurnAction,
};

const MAX_VISIBLE_USER_CHARS: usize = 4_000;
const MAX_RETURNED_ACTIONS_PER_TURN: i64 = 100;
const MAX_COUNTED_MESSAGES_PER_SESSION: i64 = 10_000;
const SESSION_SCAN_ROWS_PER_RESULT: i64 = 64;
const MAX_SESSION_SCAN_ROWS: i64 = 12_800;
const ACTIVITY_CURSOR_PREFIX: &str = "sa2_";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActivitySessionCursor {
    version: u8,
    project_filter: Option<String>,
    last_epoch: i64,
    last_row_id: i64,
}

pub fn list_activity_sessions(
    conn: &Connection,
    project: Option<&str>,
    cursor: Option<&str>,
    limit: i64,
) -> Result<RawSessionActivityPage> {
    let cursor = cursor.map(decode_activity_cursor).transpose()?;
    if let Some(cursor) = cursor.as_ref() {
        if cursor.version != 2 || cursor.project_filter.as_deref() != project {
            bail!("session activity cursor does not match the requested filters");
        }
    }
    let scan_limit = limit
        .saturating_mul(SESSION_SCAN_ROWS_PER_RESULT)
        .clamp(SESSION_SCAN_ROWS_PER_RESULT, MAX_SESSION_SCAN_ROWS);
    let fetch_limit = scan_limit.saturating_add(1);
    let project_predicate = project
        .map(|_| "r.project = :project AND ")
        .unwrap_or_default();
    let sql = format!(
        "WITH candidates AS MATERIALIZED (
           SELECT r.id, r.source_root, r.project, r.session_id,
                  r.created_at_epoch
           FROM raw_messages r
           WHERE {project_predicate}(:has_cursor = 0
               OR r.created_at_epoch < :last_epoch
               OR (r.created_at_epoch = :last_epoch AND r.id < :last_row_id))
           ORDER BY r.created_at_epoch DESC, r.id DESC
           LIMIT :fetch_limit
         )
         SELECT c.id, c.source_root, c.project, c.session_id,
                c.created_at_epoch,
                NOT EXISTS (
                  SELECT 1 FROM raw_messages newer
                  WHERE newer.source_root = c.source_root
                    AND newer.project = c.project
                    AND newer.session_id = c.session_id
                    AND (newer.created_at_epoch > c.created_at_epoch
                      OR (newer.created_at_epoch = c.created_at_epoch
                          AND newer.id > c.id))
                )
         FROM candidates c
         ORDER BY c.created_at_epoch DESC, c.id DESC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let has_cursor = i64::from(cursor.is_some());
    let last_epoch = cursor.as_ref().map(|value| value.last_epoch);
    let last_row_id = cursor.as_ref().map(|value| value.last_row_id);
    let candidates = if let Some(project) = project {
        stmt.query_map(
            rusqlite::named_params! {
                ":project": project,
                ":has_cursor": has_cursor,
                ":last_epoch": last_epoch,
                ":last_row_id": last_row_id,
                ":fetch_limit": fetch_limit,
            },
            map_activity_candidate,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        stmt.query_map(
            rusqlite::named_params! {
                ":has_cursor": has_cursor,
                ":last_epoch": last_epoch,
                ":last_row_id": last_row_id,
                ":fetch_limit": fetch_limit,
            },
            map_activity_candidate,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let raw_has_more = candidates.len() as i64 > scan_limit;
    let candidate_count = candidates.len().min(scan_limit as usize);
    let mut data = Vec::with_capacity(limit as usize);
    let mut last_scanned = None;
    let mut stopped_early = false;
    for (index, (row_id, source_root, candidate_project, session_id, epoch, is_latest)) in
        candidates.into_iter().take(candidate_count).enumerate()
    {
        last_scanned = Some((epoch, row_id));
        if is_latest {
            data.push(load_raw_session_activity(
                conn,
                &source_root,
                &candidate_project,
                &session_id,
                epoch,
            )?);
        }
        if data.len() as i64 >= limit {
            stopped_early = index + 1 < candidate_count;
            break;
        }
    }
    let has_more = stopped_early || raw_has_more;
    let next_cursor = if has_more {
        let (last_epoch, last_row_id) = last_scanned
            .context("session activity pagination lost its scanned continuation row")?;
        Some(encode_activity_cursor(&ActivitySessionCursor {
            version: 2,
            project_filter: project.map(str::to_string),
            last_epoch,
            last_row_id,
        })?)
    } else {
        None
    };
    Ok(RawSessionActivityPage {
        data,
        has_more,
        next_cursor,
    })
}

type ActivityCandidate = (i64, String, String, String, i64, bool);

fn map_activity_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityCandidate> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn load_raw_session_activity(
    conn: &Connection,
    source_root: &str,
    project: &str,
    session_id: &str,
    last_epoch: i64,
) -> Result<RawSessionActivity> {
    conn.query_row(
        "WITH bounded AS MATERIALIZED (
           SELECT id, role, created_at_epoch
           FROM raw_messages
           WHERE source_root = ?1 AND project = ?2 AND session_id = ?3
           ORDER BY created_at_epoch DESC, id DESC
           LIMIT ?4
         ), sample AS MATERIALIZED (
           SELECT role, created_at_epoch FROM bounded
           ORDER BY created_at_epoch DESC, id DESC
           LIMIT ?5
         )
         SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN role = 'assistant' THEN 1 ELSE 0 END), 0),
                MIN(created_at_epoch),
                (SELECT COUNT(*) FROM bounded) > ?5,
                (SELECT COUNT(*) FROM session_turns t
                 WHERE t.source_root = ?1 AND t.project = ?2
                   AND t.session_id = ?3)
         FROM sample",
        params![
            source_root,
            project,
            session_id,
            MAX_COUNTED_MESSAGES_PER_SESSION + 1,
            MAX_COUNTED_MESSAGES_PER_SESSION
        ],
        |row| {
            let counts_truncated = row.get(4)?;
            Ok(RawSessionActivity {
                source_root: source_root.to_string(),
                project: project.to_string(),
                session_id: session_id.to_string(),
                message_count: row.get(0)?,
                user_message_count: row.get(1)?,
                assistant_message_count: row.get(2)?,
                first_epoch: if counts_truncated { None } else { row.get(3)? },
                last_epoch,
                message_counts_truncated: counts_truncated,
                projected_turn_count: row.get(5)?,
            })
        },
    )
    .map_err(Into::into)
}

pub fn list_turns(
    conn: &Connection,
    project: Option<&str>,
    source_root: Option<&str>,
    session_id: Option<&str>,
    before_id: Option<i64>,
    limit: i64,
) -> Result<SessionActivityPage> {
    let fetch_limit = limit.saturating_add(1);
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
            params![project, source_root, session_id, before_id, fetch_limit],
            map_activity_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut data = rows
        .into_iter()
        .map(|mut item| {
            let turn_id = item.turn.id.context("stored turn is missing id")?;
            let (actions, truncated) = load_actions(conn, turn_id)?;
            item.turn.actions = actions;
            item.turn.actions_truncated = truncated;
            Ok(item)
        })
        .collect::<Result<Vec<_>>>()?;
    let has_more = data.len() as i64 > limit;
    data.truncate(limit as usize);
    let next_before_id = has_more
        .then(|| data.last().and_then(|item| item.turn.id))
        .flatten();
    Ok(SessionActivityPage {
        data,
        has_more,
        next_before_id,
    })
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
        let (actions, truncated) = load_actions(conn, id)?;
        item.turn.actions = actions;
        item.turn.actions_truncated = truncated;
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
            understanding: safe_optional_text(row.get(8)?),
            understanding_source: row.get(9)?,
            result_message_id: row.get(10)?,
            actions_summary: safe_optional_text(row.get(11)?),
            result_status: row.get(12)?,
            result_summary: safe_optional_text(row.get(13)?),
            started_at_epoch: row.get(14)?,
            ended_at_epoch: row.get(15)?,
            capture_health: row.get(16)?,
            actions: Vec::new(),
            actions_truncated: false,
        },
    })
}

fn bounded_visible_text(value: String) -> String {
    let value = crate::adapter::common::redact_sensitive_text(&value);
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

fn safe_optional_text(value: Option<String>) -> Option<String> {
    value.map(|text| crate::adapter::common::redact_sensitive_text(&text))
}

fn load_actions(conn: &Connection, turn_id: i64) -> Result<(Vec<TurnAction>, bool)> {
    let mut stmt = conn.prepare(
        "SELECT action_index, kind, tool_name, summary, event_row_id,
                files_json, outcome, created_at_epoch
         FROM session_turn_actions WHERE session_turn_id = ?1
         ORDER BY action_index LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![turn_id, MAX_RETURNED_ACTIONS_PER_TURN + 1], |row| {
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
    let mut actions = rows
        .into_iter()
        .map(
            |(index, kind, tool_name, summary, event_row_id, files_json, outcome, created)| {
                let files = serde_json::from_str(&files_json)
                    .context("decode session turn action files_json")?;
                Ok(TurnAction {
                    index,
                    kind,
                    tool_name: safe_optional_text(tool_name),
                    summary: crate::adapter::common::redact_sensitive_text(&summary),
                    event_row_id,
                    files,
                    outcome,
                    created_at_epoch: created,
                })
            },
        )
        .collect::<Result<Vec<_>>>()?;
    let truncated = actions.len() as i64 > MAX_RETURNED_ACTIONS_PER_TURN;
    actions.truncate(MAX_RETURNED_ACTIONS_PER_TURN as usize);
    Ok((actions, truncated))
}

fn encode_activity_cursor(cursor: &ActivitySessionCursor) -> Result<String> {
    let bytes = serde_json::to_vec(cursor)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(ACTIVITY_CURSOR_PREFIX.len() + bytes.len() * 2);
    encoded.push_str(ACTIVITY_CURSOR_PREFIX);
    for byte in bytes {
        encoded.push(char::from(HEX[(byte >> 4) as usize]));
        encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    Ok(encoded)
}

fn decode_activity_cursor(encoded: &str) -> Result<ActivitySessionCursor> {
    let payload = encoded
        .strip_prefix(ACTIVITY_CURSOR_PREFIX)
        .context("invalid session activity cursor prefix")?;
    if payload.is_empty() || payload.len() > 4_096 || payload.len() % 2 != 0 {
        bail!("invalid session activity cursor encoding");
    }
    let mut bytes = Vec::with_capacity(payload.len() / 2);
    for pair in payload.as_bytes().chunks_exact(2) {
        let high =
            decode_hex_nibble(pair[0]).context("invalid session activity cursor encoding")?;
        let low = decode_hex_nibble(pair[1]).context("invalid session activity cursor encoding")?;
        bytes.push((high << 4) | low);
    }
    serde_json::from_slice(&bytes).context("invalid session activity cursor payload")
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
