use std::collections::BTreeSet;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::types::{ProjectionResult, SessionActivityKey, SessionTurn, TurnAction};

pub const PROJECTION_VERSION: i64 = 1;
const MAX_DERIVED_TEXT_BYTES: usize = 600;
const MAX_PROJECT_MESSAGES: usize = 10_000;
const MAX_PROJECT_ACTIONS: usize = 20_000;

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
    reference_time_epoch: Option<i64>,
}

impl CapturedAction {
    fn effective_epoch(&self) -> i64 {
        self.reference_time_epoch.unwrap_or(self.created_at_epoch)
    }
}

pub fn project_session(
    conn: &mut Connection,
    key: &SessionActivityKey,
    now_epoch: i64,
) -> Result<ProjectionResult> {
    validate_key(key)?;
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("begin session activity projection snapshot")?;
    let messages = load_messages(&tx, key)?;
    if messages.iter().all(|message| message.role != "user") {
        bail!("raw session contains no user message occurrences");
    }

    let transcript_identity_id = single_transcript_identity(&messages)?;
    let session_row_id = resolve_session_row_id(&tx, key, transcript_identity_id)?;
    let captured_actions = match session_row_id {
        Some(id) => load_captured_actions(&tx, id)?,
        None => Vec::new(),
    };
    let source_digest = source_digest(
        key,
        transcript_identity_id,
        session_row_id,
        &messages,
        &captured_actions,
    );

    if projection_is_current(&tx, key, &source_digest)? {
        let turn_count = count_turns(&tx, key)? as usize;
        tx.commit()
            .context("commit unchanged session activity projection snapshot")?;
        return Ok(ProjectionResult {
            changed: false,
            source_digest,
            turn_count,
        });
    }

    let turns = build_turns(&messages, &captured_actions, session_row_id.is_some());
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
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
             ON CONFLICT(source_root, project, session_id, turn_index) DO UPDATE SET
                 transcript_identity_id = excluded.transcript_identity_id,
                 session_row_id = excluded.session_row_id,
                 user_message_id = excluded.user_message_id,
                 understanding_message_id = excluded.understanding_message_id,
                 result_message_id = excluded.result_message_id,
                 understanding = excluded.understanding,
                 understanding_source = excluded.understanding_source,
                 actions_summary = excluded.actions_summary,
                 actions_summary_source = excluded.actions_summary_source,
                 result_status = excluded.result_status,
                 result_summary = excluded.result_summary,
                 started_at_epoch = excluded.started_at_epoch,
                 ended_at_epoch = excluded.ended_at_epoch,
                 capture_health = excluded.capture_health,
                 source_digest = excluded.source_digest,
                 projection_version = excluded.projection_version,
                 updated_at_epoch = excluded.updated_at_epoch",
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
        let turn_id: i64 = tx.query_row(
            "SELECT id FROM session_turns
             WHERE source_root = ?1 AND project = ?2 AND session_id = ?3
               AND turn_index = ?4",
            params![
                key.source_root,
                key.project,
                key.session_id,
                turn.turn_index
            ],
            |row| row.get(0),
        )?;
        tx.execute(
            "DELETE FROM session_turn_actions WHERE session_turn_id = ?1",
            [turn_id],
        )?;
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
    tx.execute(
        "DELETE FROM session_turns
         WHERE source_root = ?1 AND project = ?2 AND session_id = ?3
           AND turn_index > ?4",
        params![
            key.source_root,
            key.project,
            key.session_id,
            turns.len() as i64
        ],
    )?;
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
    if key.source_root.len() > 1_024 || key.project.len() > 4_096 || key.session_id.len() > 512 {
        bail!("session activity identity exceeds the supported length");
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
           id
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![
            key.source_root,
            key.project,
            key.session_id,
            (MAX_PROJECT_MESSAGES + 1) as i64
        ],
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
    let mut messages = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if messages.len() > MAX_PROJECT_MESSAGES {
        bail!("raw session exceeds the projection message limit of {MAX_PROJECT_MESSAGES}");
    }
    messages.shrink_to_fit();
    Ok(messages)
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

fn resolve_session_row_id(
    conn: &Connection,
    key: &SessionActivityKey,
    transcript_identity_id: Option<i64>,
) -> Result<Option<i64>> {
    if key.source_root != crate::memory::raw_archive::SOURCE_ROOT_LOCAL {
        return Ok(None);
    }
    let Some(transcript_identity_id) = transcript_identity_id else {
        return Ok(None);
    };
    let transcript_path = conn
        .query_row(
            "SELECT transcript_path FROM raw_session_identities
             WHERE id = ?1 AND source_root = ?2 AND project = ?3
               AND canonical_session_id = ?4 AND status = 'active'",
            params![
                transcript_identity_id,
                key.source_root,
                key.project,
                key.session_id
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(host_name) = transcript_path.as_deref().and_then(transcript_host) else {
        return Ok(None);
    };
    let Some(project_id) =
        crate::project_alias::resolve_project_identity(conn, &key.project)?.canonical_project_id
    else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(
        "SELECT s.id
         FROM sessions s
         JOIN hosts h ON h.id = s.host_id
         WHERE s.project_id = ?1 AND s.session_id = ?2 AND h.name = ?3
         ORDER BY s.id LIMIT 2",
    )?;
    let ids = stmt
        .query_map(params![project_id, key.session_id, host_name], |row| {
            row.get(0)
        })?
        .collect::<rusqlite::Result<Vec<i64>>>()?;
    Ok(match ids.as_slice() {
        [id] => Some(*id),
        _ => None,
    })
}

fn transcript_host(path: &str) -> Option<&'static str> {
    let normalized = path.replace('\\', "/");
    let segments = normalized.split('/').collect::<BTreeSet<_>>();
    let matches = [
        (".claude", "claude-code"),
        (".codex", "codex-cli"),
        (".cursor", "cursor"),
    ]
    .into_iter()
    .filter_map(|(segment, host)| segments.contains(segment).then_some(host))
    .collect::<Vec<_>>();
    matches
        .as_slice()
        .first()
        .copied()
        .filter(|_| matches.len() == 1)
}

fn load_captured_actions(conn: &Connection, session_row_id: i64) -> Result<Vec<CapturedAction>> {
    let mut stmt = conn.prepare(
        "SELECT id, event_type, tool_name, content_hash, created_at_epoch,
                reference_time_epoch
         FROM captured_events
         WHERE session_row_id = ?1
           AND (tool_name IS NOT NULL OR event_type IN
                ('file_edit', 'file_create', 'file_write', 'search', 'bash',
                 'tool_result', 'cursor_tool_failure'))
         ORDER BY COALESCE(reference_time_epoch, created_at_epoch), id
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        params![session_row_id, (MAX_PROJECT_ACTIONS + 1) as i64],
        |row| {
            Ok(CapturedAction {
                id: row.get(0)?,
                event_type: row.get(1)?,
                tool_name: row.get(2)?,
                content_hash: row.get(3)?,
                created_at_epoch: row.get(4)?,
                reference_time_epoch: row.get(5)?,
            })
        },
    )?;
    let mut actions = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    if actions.len() > MAX_PROJECT_ACTIONS {
        bail!("session exceeds the projection action limit of {MAX_PROJECT_ACTIONS}");
    }
    actions.shrink_to_fit();
    Ok(actions)
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

    let boundary_epochs = user_positions
        .iter()
        .map(|&position| messages[position].created_at_epoch)
        .collect::<BTreeSet<_>>();
    let ambiguous_epochs = actions
        .iter()
        .map(CapturedAction::effective_epoch)
        .filter(|epoch| boundary_epochs.contains(epoch))
        .collect::<BTreeSet<_>>();
    let usable_actions = actions
        .iter()
        .filter(|action| !ambiguous_epochs.contains(&action.effective_epoch()))
        .collect::<Vec<_>>();
    let mut action_cursor = 0usize;
    let mut turns = Vec::with_capacity(user_positions.len());

    for (turn_offset, &start) in user_positions.iter().enumerate() {
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
        while action_cursor < usable_actions.len()
            && usable_actions[action_cursor].effective_epoch() <= user.created_at_epoch
        {
            action_cursor += 1;
        }
        let action_start = action_cursor;
        while action_cursor < usable_actions.len()
            && next_started_at
                .map(|end_epoch| usable_actions[action_cursor].effective_epoch() < end_epoch)
                .unwrap_or(true)
        {
            action_cursor += 1;
        }
        let turn_action_sources = &usable_actions[action_start..action_cursor];
        let ambiguous_action_boundary = ambiguous_epochs.contains(&user.created_at_epoch)
            || next_started_at.is_some_and(|epoch| ambiguous_epochs.contains(&epoch));
        let turn_actions = turn_action_sources
            .iter()
            .enumerate()
            .map(|(index, action)| project_action(index as i64 + 1, action))
            .collect::<Vec<_>>();
        let first_action_epoch = turn_action_sources
            .first()
            .map(|action| action.effective_epoch());
        let understanding_message = assistant_messages.iter().copied().find(|message| {
            first_action_epoch
                .map(|epoch| {
                    message.event_time_source == "transcript_event"
                        && message.created_at_epoch < epoch
                })
                .unwrap_or(true)
                && meaningful_understanding(&message.content)
        });
        let last_action_epoch = turn_action_sources
            .last()
            .map(|action| action.effective_epoch());
        let result_message = if let Some(last_action_epoch) = last_action_epoch {
            assistant_messages.iter().rev().copied().find(|message| {
                message.event_time_source == "transcript_event"
                    && message.created_at_epoch > last_action_epoch
            })
        } else {
            assistant_messages.last().copied()
        };
        let result_summary = result_message.map(|message| bounded_text(&message.content));
        let result_status = classify_result(result_summary.as_deref(), &turn_actions);
        let actions_summary = summarize_actions(&turn_actions);
        let precise_time = user.event_time_source == "transcript_event"
            && turn_action_sources
                .iter()
                .all(|action| action.reference_time_epoch.is_some())
            && result_message
                .map(|message| message.event_time_source == "transcript_event")
                .unwrap_or(false);
        let capture_health = if has_session_link && precise_time && !ambiguous_action_boundary {
            "partial"
        } else {
            "unavailable"
        };

        turns.push(SessionTurn {
            id: None,
            turn_index: turn_offset as i64 + 1,
            user_message_id: user.id,
            user_said: bounded_text(&user.content),
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
            actions_truncated: false,
        });
    }
    turns
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
    let redacted = crate::adapter::common::redact_sensitive_text(text);
    let trimmed = redacted.trim();
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
    let safe_label = crate::adapter::common::redact_sensitive_text(label);
    let outcome = action
        .event_type
        .contains("failure")
        .then(|| "failed".to_string());
    TurnAction {
        index,
        kind: kind.to_string(),
        tool_name: action
            .tool_name
            .as_deref()
            .map(crate::adapter::common::redact_sensitive_text),
        summary: format!("Captured {safe_label} activity"),
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
    transcript_identity_id: Option<i64>,
    session_row_id: Option<i64>,
    messages: &[RawMessage],
    actions: &[CapturedAction],
) -> String {
    let mut hasher = Sha256::new();
    for value in [&key.source_root, &key.project, &key.session_id] {
        feed(&mut hasher, value.as_bytes());
    }
    feed(
        &mut hasher,
        &transcript_identity_id.unwrap_or(-1).to_be_bytes(),
    );
    feed(&mut hasher, &session_row_id.unwrap_or(-1).to_be_bytes());
    for message in messages {
        feed(&mut hasher, &message.id.to_be_bytes());
        feed(&mut hasher, message.role.as_bytes());
        feed(&mut hasher, message.content_hash.as_bytes());
        feed(&mut hasher, &message.created_at_epoch.to_be_bytes());
        feed(&mut hasher, message.event_time_source.as_bytes());
        feed(
            &mut hasher,
            &message.transcript_identity_id.unwrap_or(-1).to_be_bytes(),
        );
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
        feed(
            &mut hasher,
            action.tool_name.as_deref().unwrap_or_default().as_bytes(),
        );
        feed(&mut hasher, action.content_hash.as_bytes());
        feed(&mut hasher, &action.created_at_epoch.to_be_bytes());
        feed(
            &mut hasher,
            &action.reference_time_epoch.unwrap_or(-1).to_be_bytes(),
        );
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
