//! Transport-neutral raw query bounds and JSON response contracts (GH720).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::raw_archive::RawMessage;

const RAW_ARCHIVE_NOTE: &str = "raw archive rows are captured chat turns, not curated memories";
pub(crate) const RAW_SESSION_MESSAGES_DEFAULT_LIMIT: i64 = 500;
pub(crate) const RAW_SESSION_MESSAGES_MAX_LIMIT: i64 = 2_000;
const RAW_SESSION_MESSAGES_ORDER: &str = "created_at_epoch_asc_id_asc";
const RAW_SESSION_CURSOR_PREFIX: &str = "rm1_";

pub(crate) fn parse_time_lower_bound(value: &str) -> Result<i64> {
    parse_time_bound(value, DateBound::Lower)
}

pub(crate) fn parse_time_upper_bound(value: &str) -> Result<i64> {
    parse_time_bound(value, DateBound::Upper)
}

#[derive(Clone, Copy)]
enum DateBound {
    Lower,
    Upper,
}

fn parse_time_bound(value: &str, date_bound: DateBound) -> Result<i64> {
    let trimmed = value.trim();
    if let Ok(epoch) = trimmed.parse::<i64>() {
        return Ok(epoch);
    }
    if let Ok(datetime) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Ok(datetime.timestamp());
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let time = match date_bound {
            DateBound::Lower => (0, 0, 0),
            DateBound::Upper => (23, 59, 59),
        };
        let datetime = date
            .and_hms_opt(time.0, time.1, time.2)
            .ok_or_else(|| anyhow::anyhow!("invalid UTC day boundary for {trimmed:?}"))?;
        return Ok(datetime.and_utc().timestamp());
    }
    anyhow::bail!(
        "invalid time bound {trimmed:?}: expected Unix epoch, ISO8601 datetime, or YYYY-MM-DD"
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_raw_search_json(
    query: &str,
    project: Option<&str>,
    branch: Option<&str>,
    role: Option<&str>,
    limit: i64,
    offset: i64,
    since_epoch: Option<i64>,
    until_epoch: Option<i64>,
    has_more: bool,
    rows: &[RawMessage],
) -> RawSearchJson {
    let normalized_limit = limit.max(1);
    let normalized_offset = offset.max(0);
    RawSearchJson {
        query: query.to_string(),
        project: project.map(str::to_string),
        branch: branch.map(str::to_string),
        role: role.map(str::to_string),
        limit: normalized_limit,
        offset: normalized_offset,
        since_epoch,
        until_epoch,
        source_type: "raw_archive".to_string(),
        note: RAW_ARCHIVE_NOTE.to_string(),
        count: rows.len(),
        has_more,
        next_offset: has_more.then_some(normalized_offset.saturating_add(normalized_limit)),
        results: rows.iter().map(RawArchiveRowJson::from).collect(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RawSearchJson {
    pub query: String,
    pub project: Option<String>,
    pub branch: Option<String>,
    pub role: Option<String>,
    pub limit: i64,
    pub offset: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until_epoch: Option<i64>,
    pub source_type: String,
    pub note: String,
    pub count: usize,
    pub has_more: bool,
    pub next_offset: Option<i64>,
    pub results: Vec<RawArchiveRowJson>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RawArchiveRowJson {
    pub id: i64,
    pub source_type: String,
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub content: String,
    pub source: String,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
}

impl From<&RawMessage> for RawArchiveRowJson {
    fn from(row: &RawMessage) -> Self {
        Self {
            id: row.id,
            source_type: "raw_archive".to_string(),
            session_id: row.session_id.clone(),
            project: row.project.clone(),
            role: row.role.clone(),
            content: row.content.clone(),
            source: row.source.clone(),
            branch: row.branch.clone(),
            cwd: row.cwd.clone(),
            created_at_epoch: row.created_at_epoch,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RawSessionMessagesRequest {
    pub source_root: String,
    pub project: String,
    pub session_id: String,
    pub limit: i64,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RawSessionMessagesJson {
    pub source_type: String,
    pub source_root: String,
    pub project: String,
    pub session_id: String,
    pub limit: i64,
    pub count: usize,
    pub order: String,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub messages: Vec<RawSessionMessageJson>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RawSessionMessageJson {
    pub id: i64,
    pub role: String,
    pub content: String,
    pub source: String,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
}

impl From<&RawMessage> for RawSessionMessageJson {
    fn from(row: &RawMessage) -> Self {
        Self {
            id: row.id,
            role: row.role.clone(),
            content: row.content.clone(),
            source: row.source.clone(),
            branch: row.branch.clone(),
            cwd: row.cwd.clone(),
            created_at_epoch: row.created_at_epoch,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RawSessionCursor {
    version: u8,
    source_root: String,
    project: String,
    session_id: String,
    snapshot_max_id: i64,
    last_created_at_epoch: i64,
    last_id: i64,
}

pub(crate) fn query_raw_session_messages(
    conn: &Connection,
    request: &RawSessionMessagesRequest,
) -> Result<RawSessionMessagesJson> {
    let limit = request.limit.clamp(1, RAW_SESSION_MESSAGES_MAX_LIMIT);
    let cursor = request
        .cursor
        .as_deref()
        .map(decode_raw_session_cursor)
        .transpose()?;

    if let Some(cursor) = cursor.as_ref() {
        validate_cursor(conn, cursor, request)?;
    }

    let snapshot_max_id = match cursor.as_ref() {
        Some(cursor) => Some(cursor.snapshot_max_id),
        None => conn
            .query_row(
                "SELECT MAX(id) FROM raw_messages \
                 WHERE source_root = ?1 AND project = ?2 AND session_id = ?3",
                params![request.source_root, request.project, request.session_id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten(),
    };

    let mut rows = match snapshot_max_id {
        None => Vec::new(),
        Some(snapshot_max_id) => query_snapshot_page(
            conn,
            request,
            snapshot_max_id,
            cursor.as_ref(),
            limit.saturating_add(1),
        )?,
    };
    let has_more = rows.len() as i64 > limit;
    rows.truncate(limit as usize);
    let next_cursor = if has_more {
        let last = rows
            .last()
            .context("raw messages pagination returned no continuation row")?;
        Some(encode_raw_session_cursor(&RawSessionCursor {
            version: 1,
            source_root: request.source_root.clone(),
            project: request.project.clone(),
            session_id: request.session_id.clone(),
            snapshot_max_id: snapshot_max_id
                .context("raw messages pagination lost its snapshot boundary")?,
            last_created_at_epoch: last.created_at_epoch,
            last_id: last.id,
        })?)
    } else {
        None
    };

    Ok(RawSessionMessagesJson {
        source_type: "raw_archive".to_string(),
        source_root: request.source_root.clone(),
        project: request.project.clone(),
        session_id: request.session_id.clone(),
        limit,
        count: rows.len(),
        order: RAW_SESSION_MESSAGES_ORDER.to_string(),
        has_more,
        next_cursor,
        messages: rows.iter().map(RawSessionMessageJson::from).collect(),
    })
}

fn query_snapshot_page(
    conn: &Connection,
    request: &RawSessionMessagesRequest,
    snapshot_max_id: i64,
    cursor: Option<&RawSessionCursor>,
    fetch_limit: i64,
) -> Result<Vec<RawMessage>> {
    let mut rows = Vec::new();
    if let Some(cursor) = cursor {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, project, role, content, source, branch, cwd, \
                    created_at_epoch \
             FROM raw_messages \
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3 \
               AND id <= ?4 \
               AND (created_at_epoch > ?5 \
                    OR (created_at_epoch = ?5 AND id > ?6)) \
             ORDER BY created_at_epoch ASC, id ASC LIMIT ?7",
        )?;
        let mapped = stmt.query_map(
            params![
                request.source_root,
                request.project,
                request.session_id,
                snapshot_max_id,
                cursor.last_created_at_epoch,
                cursor.last_id,
                fetch_limit
            ],
            map_raw_message,
        )?;
        for row in mapped {
            rows.push(row?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, session_id, project, role, content, source, branch, cwd, \
                    created_at_epoch \
             FROM raw_messages \
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3 \
               AND id <= ?4 \
             ORDER BY created_at_epoch ASC, id ASC LIMIT ?5",
        )?;
        let mapped = stmt.query_map(
            params![
                request.source_root,
                request.project,
                request.session_id,
                snapshot_max_id,
                fetch_limit
            ],
            map_raw_message,
        )?;
        for row in mapped {
            rows.push(row?);
        }
    }
    Ok(rows)
}

fn map_raw_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMessage> {
    Ok(RawMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project: row.get(2)?,
        role: row.get(3)?,
        content: row.get(4)?,
        source: row.get(5)?,
        branch: row.get(6)?,
        cwd: row.get(7)?,
        created_at_epoch: row.get(8)?,
    })
}

fn validate_cursor(
    conn: &Connection,
    cursor: &RawSessionCursor,
    request: &RawSessionMessagesRequest,
) -> Result<()> {
    if cursor.version != 1 {
        anyhow::bail!("invalid raw messages cursor version");
    }
    if cursor.source_root != request.source_root
        || cursor.project != request.project
        || cursor.session_id != request.session_id
    {
        anyhow::bail!(
            "raw messages cursor selector mismatch: cursor is bound to another source_root/project/session_id tuple"
        );
    }
    if cursor.snapshot_max_id <= 0 || cursor.last_id <= 0 || cursor.last_id > cursor.snapshot_max_id
    {
        anyhow::bail!("invalid raw messages cursor boundary");
    }
    let snapshot_exists = conn
        .query_row(
            "SELECT 1 FROM raw_messages \
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3 AND id = ?4",
            params![
                request.source_root,
                request.project,
                request.session_id,
                cursor.snapshot_max_id
            ],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !snapshot_exists {
        anyhow::bail!("invalid raw messages cursor snapshot");
    }
    let anchor_epoch = conn
        .query_row(
            "SELECT created_at_epoch FROM raw_messages \
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3 AND id = ?4",
            params![
                request.source_root,
                request.project,
                request.session_id,
                cursor.last_id
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if anchor_epoch != Some(cursor.last_created_at_epoch) {
        anyhow::bail!("invalid raw messages cursor anchor");
    }
    Ok(())
}

fn encode_raw_session_cursor(cursor: &RawSessionCursor) -> Result<String> {
    let bytes = serde_json::to_vec(cursor)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(RAW_SESSION_CURSOR_PREFIX.len() + bytes.len() * 2);
    encoded.push_str(RAW_SESSION_CURSOR_PREFIX);
    for byte in bytes {
        encoded.push(char::from(HEX[(byte >> 4) as usize]));
        encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    Ok(encoded)
}

fn decode_raw_session_cursor(encoded: &str) -> Result<RawSessionCursor> {
    let payload = encoded
        .strip_prefix(RAW_SESSION_CURSOR_PREFIX)
        .context("invalid raw messages cursor prefix")?;
    if payload.is_empty() || payload.len() % 2 != 0 {
        anyhow::bail!("invalid raw messages cursor encoding");
    }
    let mut bytes = Vec::with_capacity(payload.len() / 2);
    for pair in payload.as_bytes().chunks_exact(2) {
        let high = decode_hex_nibble(pair[0]).context("invalid raw messages cursor encoding")?;
        let low = decode_hex_nibble(pair[1]).context("invalid raw messages cursor encoding")?;
        bytes.push((high << 4) | low);
    }
    serde_json::from_slice(&bytes).context("invalid raw messages cursor payload")
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod session_message_tests {
    use anyhow::{Context, Result};

    use super::{
        encode_raw_session_cursor, query_raw_session_messages, RawSessionCursor,
        RawSessionMessagesRequest, RAW_SESSION_MESSAGES_MAX_LIMIT, RAW_SESSION_MESSAGES_ORDER,
    };
    use crate::memory::raw_archive::{
        insert_raw_message_from_root_at, ROLE_ASSISTANT, ROLE_USER, SOURCE_TRANSCRIPT,
    };

    fn request(
        source_root: &str,
        project: &str,
        session_id: &str,
        limit: i64,
        cursor: Option<String>,
    ) -> RawSessionMessagesRequest {
        RawSessionMessagesRequest {
            source_root: source_root.to_string(),
            project: project.to_string(),
            session_id: session_id.to_string(),
            limit,
            cursor,
        }
    }

    fn insert(
        conn: &rusqlite::Connection,
        source_root: &str,
        project: &str,
        session_id: &str,
        content: &str,
        epoch: i64,
    ) -> Result<i64> {
        Ok(insert_raw_message_from_root_at(
            conn,
            session_id,
            project,
            ROLE_USER,
            content,
            SOURCE_TRANSCRIPT,
            Some("main"),
            Some(project),
            source_root,
            Some(epoch),
        )?
        .context("test raw row must be inserted")?
        .id)
    }

    #[test]
    fn exact_identity_order_and_full_content_are_preserved() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let long = "x".repeat(260);
        let first_id = insert(&conn, "root-a", "/repo", "s1", &long, 10)?;
        let second_id = insert(&conn, "root-a", "/repo", "s1", "second", 10)?;
        insert(&conn, "root-b", "/repo", "s1", "other root", 5)?;
        insert(&conn, "root-a", "/other", "s1", "other project", 5)?;
        insert(&conn, "root-a", "/repo", "s2", "other session", 5)?;

        let page = query_raw_session_messages(&conn, &request("root-a", "/repo", "s1", 500, None))?;
        assert_eq!(page.messages.len(), 2);
        assert_eq!(page.messages[0].id, first_id);
        assert_eq!(page.messages[1].id, second_id);
        assert_eq!(page.messages[0].content, long);
        assert_eq!(page.order, RAW_SESSION_MESSAGES_ORDER);
        assert!(!page.has_more);
        assert!(page.next_cursor.is_none());
        Ok(())
    }

    #[test]
    fn snapshot_cursor_has_no_gaps_duplicates_or_post_page_insertions() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        for index in 0..5 {
            insert(
                &conn,
                "root-a",
                "/repo",
                "s1",
                &format!("message-{index}"),
                10 + i64::from(index / 2),
            )?;
        }

        let first = query_raw_session_messages(&conn, &request("root-a", "/repo", "s1", 2, None))?;
        assert!(first.has_more);
        let cursor = first.next_cursor.context("first page must continue")?;
        insert(&conn, "root-a", "/repo", "s1", "backdated late insert", 0)?;

        let second =
            query_raw_session_messages(&conn, &request("root-a", "/repo", "s1", 2, Some(cursor)))?;
        let third = query_raw_session_messages(
            &conn,
            &request("root-a", "/repo", "s1", 2, second.next_cursor.clone()),
        )?;
        let contents: Vec<&str> = first
            .messages
            .iter()
            .chain(second.messages.iter())
            .chain(third.messages.iter())
            .map(|row| row.content.as_str())
            .collect();
        assert_eq!(
            contents,
            vec![
                "message-0",
                "message-1",
                "message-2",
                "message-3",
                "message-4"
            ]
        );
        assert!(!contents.contains(&"backdated late insert"));
        assert!(!third.has_more);
        Ok(())
    }

    #[test]
    fn empty_invalid_mismatched_and_bounded_requests_are_explicit() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let empty =
            query_raw_session_messages(&conn, &request("root-a", "/repo", "missing", 9_999, None))?;
        assert_eq!(empty.limit, RAW_SESSION_MESSAGES_MAX_LIMIT);
        assert!(empty.messages.is_empty());

        let invalid = query_raw_session_messages(
            &conn,
            &request("root-a", "/repo", "s1", 2, Some("bad".to_string())),
        );
        assert!(invalid.is_err());

        let first_id = insert(&conn, "root-a", "/repo", "s1", "one", 1)?;
        let second_id = insert(&conn, "root-a", "/repo", "s1", "two", 2)?;
        let first = query_raw_session_messages(&conn, &request("root-a", "/repo", "s1", 1, None))?;
        let mismatch = query_raw_session_messages(
            &conn,
            &request("root-b", "/repo", "s1", 1, first.next_cursor),
        );
        assert!(mismatch.is_err());

        let impossible_boundary = encode_raw_session_cursor(&RawSessionCursor {
            version: 1,
            source_root: "root-a".to_string(),
            project: "/repo".to_string(),
            session_id: "s1".to_string(),
            snapshot_max_id: first_id,
            last_created_at_epoch: 2,
            last_id: second_id,
        })?;
        let impossible = query_raw_session_messages(
            &conn,
            &request("root-a", "/repo", "s1", 1, Some(impossible_boundary)),
        );
        assert!(impossible.is_err());

        let wrong_anchor = encode_raw_session_cursor(&RawSessionCursor {
            version: 1,
            source_root: "root-a".to_string(),
            project: "/repo".to_string(),
            session_id: "s1".to_string(),
            snapshot_max_id: second_id,
            last_created_at_epoch: 999,
            last_id: first_id,
        })?;
        let wrong_anchor_result = query_raw_session_messages(
            &conn,
            &request("root-a", "/repo", "s1", 1, Some(wrong_anchor)),
        );
        assert!(wrong_anchor_result.is_err());
        Ok(())
    }

    #[test]
    fn json_rows_keep_nullable_metadata_and_machine_fields() -> Result<()> {
        let conn = rusqlite::Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        insert_raw_message_from_root_at(
            &conn,
            "s1",
            "/repo",
            ROLE_ASSISTANT,
            "assistant output",
            SOURCE_TRANSCRIPT,
            None,
            None,
            "root-a",
            Some(7),
        )?;
        let page = query_raw_session_messages(&conn, &request("root-a", "/repo", "s1", 500, None))?;
        let json = serde_json::to_value(page)?;
        assert_eq!(json["source_type"], "raw_archive");
        assert_eq!(json["source_root"], "root-a");
        assert_eq!(json["project"], "/repo");
        assert_eq!(json["session_id"], "s1");
        assert_eq!(json["count"], 1);
        assert_eq!(json["order"], RAW_SESSION_MESSAGES_ORDER);
        assert_eq!(json["messages"][0]["role"], ROLE_ASSISTANT);
        assert_eq!(json["messages"][0]["content"], "assistant output");
        assert!(json["messages"][0]["branch"].is_null());
        assert!(json["messages"][0]["cwd"].is_null());
        assert_eq!(json["messages"][0]["created_at_epoch"], 7);
        Ok(())
    }
}
