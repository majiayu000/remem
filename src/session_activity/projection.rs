use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use super::types::{ProjectionResult, SessionActivityKey, SessionTurn, TurnAction};

pub const PROJECTION_VERSION: i64 = 1;
const MAX_DERIVED_TEXT_BYTES: usize = 600;

#[derive(Debug)]
struct RawMessage {
    id: i64,
    role: String,
    content: String,
    content_hash: String,
    created_at_epoch: i64,
    event_time_source: String,
    transcript_identity_id: Option<i64>,
    transcript_record_ordinal: Option<i64>,
}

#[derive(Debug)]
struct CapturedAction {
    id: i64,
    event_type: String,
    tool_name: Option<String>,
    content_hash: String,
    created_at_epoch: i64,
}

pub fn project_session(
    conn: &mut Connection,
    key: &SessionActivityKey,
    now_epoch: i64,
) -> Result<ProjectionResult> {
    validate_key(key)?;
    let messages = load_messages(conn, key)?;
    if messages.iter().all(|message| message.role != "user") {
        bail!("raw session contains no user message occurrences");
    }

    let transcript_identity_id = single_transcript_identity(&messages)?;
    let session_row_id = resolve_session_row_id(conn, key)?;
    let captured_actions = match session_row_id {
        Some(id) => load_captured_actions(conn, id)?,
        None => Vec::new(),
    };
    let source_digest = source_digest(key, &messages, &captured_actions);

    if projection_is_current(conn, key, &source_digest)? {
        let turn_count = count_turns(conn, key)? as usize;
        return Ok(ProjectionResult {
            changed: false,
            source_digest,
            turn_count,
        });
    }

    let turns = build_turns(&messages, &captured_actions, session_row_id.is_some());
    let tx = conn
        .transaction()
        .context("begin session activity projection")?;
    tx.execute(
        "DELETE FROM session_turns
         WHERE source_root = ?1 AND project = ?2 AND session_id = ?3",
        params![key.source_root, key.project, key.session_id],
    )?;

    for turn in &turns {
        tx.execute(
            "INSERT INTO session_turns
             (transcript_identity_id, source_root, project, session_id,
              session_row_id, turn_index, user_message_id,
              understanding_message_id, result_message_id, understanding,
              understanding_source, actions_summary, actions_summary_source,
              result_status, result_summary, started_at_epoch, ended_at_epoch,
              capture_health, source_digest, projection_version,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                transcript_identity_id,
                key.source_root,
                key.project,
                key.session_id,
                session_row_id,
                turn.turn_index,
                turn.user_message_id,
                turn.understanding_message_id,
                turn.result_message_id,
                turn.understanding,
                turn.understanding_source,
                turn.actions_summary,
                turn.actions_summary.as_ref().map(|_| "captured_events"),
                turn.result_status,
                turn.result_summary,
                turn.started_at_epoch,
                turn.ended_at_epoch,
                turn.capture_health,
                source_digest,
                PROJECTION_VERSION,
                now_epoch,
                now_epoch,
            ],
        )?;
        let turn_id = tx.last_insert_rowid();
        for action in &turn.actions {
            tx.execute(
                "INSERT INTO session_turn_actions
                 (session_turn_id, action_index, kind, tool_name, summary,
                  event_row_id, files_json, outcome, created_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    turn_id,
                    action.index,
                    action.kind,
                    action.tool_name,
                    action.summary,
                    action.event_row_id,
                    serde_json::to_string(&action.files)?,
                    action.outcome,
                    action.created_at_epoch,
                ],
            )?;
        }
    }
    tx.commit().context("commit session activity projection")?;

    Ok(ProjectionResult {
        changed: true,
        source_digest,
        turn_count: turns.len(),
    })
}

fn validate_key(key: &SessionActivityKey) -> Result<()> {
    if key.source_root.trim().is_empty()
        || key.project.trim().is_empty()
        || key.session_id.trim().is_empty()
    {
        bail!("source_root, project, and session_id must be non-empty");
    }
    Ok(())
}

fn load_messages(conn: &Connection, key: &SessionActivityKey) -> Result<Vec<RawMessage>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content, content_hash, created_at_epoch,
                event_time_source, transcript_identity_id,
                transcript_record_ordinal
         FROM raw_messages
         WHERE source_root = ?1 AND project = ?2 AND session_id = ?3
           AND role IN ('user', 'assistant')
         ORDER BY
           CASE WHEN transcript_record_ordinal IS NULL THEN 1 ELSE 0 END,
           transcript_record_ordinal,
           created_at_epoch,
           id",
    )?;
    let rows = stmt.query_map(
        params![key.source_root, key.project, key.session_id],
        |row| {
            Ok(RawMessage {
                id: row.get(0)?,
                role: row.get(1)?,
                content: row.get(2)?,
                content_hash: row.get(3)?,
                created_at_epoch: row.get(4)?,
                event_time_source: row.get(5)?,
                transcript_identity_id: row.get(6)?,
                transcript_record_ordinal: row.get(7)?,
            })
        },
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn single_transcript_identity(messages: &[RawMessage]) -> Result<Option<i64>> {
    let mut identities = messages
        .iter()
        .filter_map(|message| message.transcript_identity_id)
        .collect::<Vec<_>>();
    identities.sort_unstable();
    identities.dedup();
    if identities.len() > 1 {
        bail!("raw session tuple resolves to multiple transcript identities");
    }
    Ok(identities.first().copied())
}

fn resolve_session_row_id(conn: &Connection, key: &SessionActivityKey) -> Result<Option<i64>> {
    let mut stmt = conn.prepare(
        "SELECT s.id
         FROM sessions s JOIN projects p ON p.id = s.project_id
         WHERE p.project_key = ?1 AND s.session_id = ?2
         ORDER BY s.id LIMIT 2",
    )?;
    let ids = stmt
        .query_map(params![key.project, key.session_id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(match ids.as_slice() {
        [id] => Some(*id),
        _ => None,
    })
}

fn load_captured_actions(conn: &Connection, session_row_id: i64) -> Result<Vec<CapturedAction>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, tool_name, content_hash, created_at_epoch
         FROM captured_events
         WHERE session_row_id = ?1
           AND (tool_name IS NOT NULL OR event_type IN
                ('file_edit', 'file_create', 'file_write', 'search', 'bash',
                 'tool_result', 'cursor_tool_failure'))
         ORDER BY created_at_epoch, id",
    )?;
    let rows = stmt.query_map([session_row_id], |row| {
        Ok(CapturedAction {
            id: row.get(0)?,
            event_type: row.get(1)?,
            tool_name: row.get(2)?,
            content_hash: row.get(3)?,
            created_at_epoch: row.get(4)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn build_turns(
    messages: &[RawMessage],
    actions: &[CapturedAction],
    has_session_link: bool,
) -> Vec<SessionTurn> {
    let user_positions = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| (message.role == "user").then_some(index))
        .collect::<Vec<_>>();

    user_positions
        .iter()
        .enumerate()
        .map(|(turn_offset, &start)| {
            let end = user_positions
                .get(turn_offset + 1)
                .copied()
                .unwrap_or(messages.len());
            let user = &messages[start];
            let assistant_messages = messages[start + 1..end]
                .iter()
                .filter(|message| message.role == "assistant")
                .collect::<Vec<_>>();
            let next_started_at = user_positions
                .get(turn_offset + 1)
                .map(|&position| messages[position].created_at_epoch);
            let turn_actions = actions
                .iter()
                .filter(|action| {
                    action.created_at_epoch >= user.created_at_epoch
                        && next_started_at
                            .map(|end_epoch| action.created_at_epoch < end_epoch)
                            .unwrap_or(true)
                })
                .enumerate()
                .map(|(index, action)| project_action(index as i64 + 1, action))
                .collect::<Vec<_>>();
            let understanding_message = assistant_messages
                .iter()
                .copied()
                .find(|message| meaningful_understanding(&message.content));
            let result_message = assistant_messages.last().copied();
            let result_summary = result_message.map(|message| bounded_text(&message.content));
            let result_status = classify_result(result_summary.as_deref(), &turn_actions);
            let actions_summary = summarize_actions(&turn_actions);
            let precise_time = user.event_time_source == "transcript_event"
                && result_message
                    .map(|message| message.event_time_source == "transcript_event")
                    .unwrap_or(false);
            let capture_health = if has_session_link && precise_time {
                "partial"
            } else {
                "unavailable"
            };

            SessionTurn {
                id: None,
                turn_index: turn_offset as i64 + 1,
                user_message_id: user.id,
                user_said: user.content.clone(),
                understanding_message_id: understanding_message.map(|message| message.id),
                understanding: understanding_message.map(|message| bounded_text(&message.content)),
                understanding_source: understanding_message.map(|_| "assistant_text".to_string()),
                result_message_id: result_message.map(|message| message.id),
                actions_summary,
                result_status,
                result_summary,
                started_at_epoch: user.created_at_epoch,
                ended_at_epoch: result_message.map(|message| message.created_at_epoch),
                capture_health: capture_health.to_string(),
                actions: turn_actions,
            }
        })
        .collect()
}

fn meaningful_understanding(text: &str) -> bool {
    let value = text.trim();
    if value.chars().count() < 15 {
        return false;
    }
    let lowered = value.to_lowercase();
    ![
        "好的",
        "让我看看",
        "我来处理",
        "稍等",
        "okay",
        "let me check",
    ]
    .iter()
    .any(|prefix| lowered == *prefix || lowered.starts_with(&format!("{prefix}。")))
}

fn bounded_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= MAX_DERIVED_TEXT_BYTES {
        return trimmed.to_string();
    }
    let mut end = MAX_DERIVED_TEXT_BYTES;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &trimmed[..end])
}

fn classify_result(summary: Option<&str>, actions: &[TurnAction]) -> String {
    let Some(summary) = summary else {
        return "aborted".to_string();
    };
    if actions.is_empty() {
        return "answered".to_string();
    }
    let lowered = summary.to_lowercase();
    if [
        "失败",
        "无法",
        "仍然报错",
        "failed",
        "unable",
        "error remains",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
    {
        return "failed".to_string();
    }
    if ["部分", "尚未", "待处理", "todo", "partial", "remaining"]
        .iter()
        .any(|needle| lowered.contains(needle))
    {
        return "partial".to_string();
    }
    "done".to_string()
}

fn project_action(index: i64, action: &CapturedAction) -> TurnAction {
    let kind = match action.event_type.as_str() {
        "file_edit" | "file_write" => "edit",
        "file_create" => "create",
        "bash" => "run",
        "search" => "search",
        _ => match action.tool_name.as_deref() {
            Some("Edit" | "Write" | "NotebookEdit") => "edit",
            Some("Bash") => "run",
            Some("Grep" | "Glob") => "search",
            Some("Read") => "read",
            Some("Task") => "external",
            _ => "other",
        },
    };
    let label = action
        .tool_name
        .as_deref()
        .unwrap_or(action.event_type.as_str());
    let outcome = action
        .event_type
        .contains("failure")
        .then(|| "failed".to_string());
    TurnAction {
        index,
        kind: kind.to_string(),
        tool_name: action.tool_name.clone(),
        summary: format!("Captured {label} activity"),
        event_row_id: Some(action.id),
        files: Vec::new(),
        outcome,
        created_at_epoch: action.created_at_epoch,
    }
}

fn summarize_actions(actions: &[TurnAction]) -> Option<String> {
    if actions.is_empty() {
        return None;
    }
    let labels = actions
        .iter()
        .map(|action| action.tool_name.as_deref().unwrap_or(&action.kind))
        .take(6)
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{} captured action(s): {labels}", actions.len()))
}

fn source_digest(
    key: &SessionActivityKey,
    messages: &[RawMessage],
    actions: &[CapturedAction],
) -> String {
    let mut hasher = Sha256::new();
    for value in [&key.source_root, &key.project, &key.session_id] {
        feed(&mut hasher, value.as_bytes());
    }
    for message in messages {
        feed(&mut hasher, &message.id.to_be_bytes());
        feed(&mut hasher, message.role.as_bytes());
        feed(&mut hasher, message.content_hash.as_bytes());
        feed(&mut hasher, &message.created_at_epoch.to_be_bytes());
        feed(
            &mut hasher,
            &message
                .transcript_record_ordinal
                .unwrap_or(-1)
                .to_be_bytes(),
        );
    }
    for action in actions {
        feed(&mut hasher, &action.id.to_be_bytes());
        feed(&mut hasher, action.event_type.as_bytes());
        feed(&mut hasher, action.content_hash.as_bytes());
        feed(&mut hasher, &action.created_at_epoch.to_be_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn feed(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn projection_is_current(
    conn: &Connection,
    key: &SessionActivityKey,
    digest: &str,
) -> Result<bool> {
    let row = conn.query_row(
        "SELECT COUNT(*), MIN(source_digest), MAX(source_digest),
                MIN(projection_version), MAX(projection_version)
         FROM session_turns
         WHERE source_root = ?1 AND project = ?2 AND session_id = ?3",
        params![key.source_root, key.project, key.session_id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
            ))
        },
    )?;
    Ok(row.0 > 0
        && row.1.as_deref() == Some(digest)
        && row.2.as_deref() == Some(digest)
        && row.3 == Some(PROJECTION_VERSION)
        && row.4 == Some(PROJECTION_VERSION))
}

fn count_turns(conn: &Connection, key: &SessionActivityKey) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM session_turns
         WHERE source_root = ?1 AND project = ?2 AND session_id = ?3",
        params![key.source_root, key.project, key.session_id],
        |row| row.get(0),
    )
    .map_err(Into::into)
}
