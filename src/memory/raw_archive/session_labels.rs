use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{params_from_iter, Connection};

use super::sessions::RawSessionSummary;
use crate::memory::session_label::render_from_stored;

pub(super) fn attach_session_labels(
    conn: &Connection,
    sessions: &mut [RawSessionSummary],
) -> Result<()> {
    for session in sessions.iter_mut() {
        apply_label(session, None, None, None);
    }
    if sessions.is_empty() {
        return Ok(());
    }

    let mut by_host_key = HashMap::new();
    let mut by_memory_key = HashMap::new();
    load_summary_labels(conn, sessions, &mut by_host_key, &mut by_memory_key)?;

    for session in sessions.iter_mut() {
        let host_key = (
            session.host.clone(),
            session.project.clone(),
            session.session_id.clone(),
        );
        let stored = by_host_key
            .get(&host_key)
            .or_else(|| by_memory_key.get(&(session.session_id.clone(), session.project.clone())))
            .cloned();
        if let Some((intent, topic, source)) = stored {
            apply_label(
                session,
                intent.as_deref(),
                topic.as_deref(),
                source.as_deref(),
            );
        }
    }
    Ok(())
}

fn apply_label(
    session: &mut RawSessionSummary,
    intent: Option<&str>,
    topic: Option<&str>,
    source: Option<&str>,
) {
    let fallback = session.user_message_samples.first().map(String::as_str);
    let view = render_from_stored(Some(session.first_epoch), intent, topic, source, fallback);
    session.mmdd = view.mmdd;
    session.session_intent = view.session_intent;
    session.session_topic = view.session_topic;
    session.display_label = view.display_label;
    session.session_intent_source = view.session_intent_source;
}

fn load_summary_labels(
    conn: &Connection,
    sessions: &[RawSessionSummary],
    by_host_key: &mut HashMap<
        (String, String, String),
        (Option<String>, Option<String>, Option<String>),
    >,
    by_memory_key: &mut HashMap<(String, String), (Option<String>, Option<String>, Option<String>)>,
) -> Result<()> {
    let session_ids = sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect::<Vec<_>>();
    if session_ids.is_empty() {
        return Ok(());
    }
    let placeholders = (1..=session_ids.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT h.name, COALESCE(p.project_path, p.project_key), s.session_id,
                ss.session_intent, ss.session_topic, ss.session_intent_source
         FROM session_summaries ss
         JOIN sessions s ON s.id = ss.session_row_id
         JOIN hosts h ON h.id = s.host_id
         JOIN projects p ON p.id = s.project_id
         WHERE s.session_id IN ({placeholders})
         ORDER BY COALESCE(ss.session_intent_updated_at_epoch, ss.created_at_epoch) DESC, ss.id DESC"
    );
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(session_ids.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })?;
    for row in rows {
        let (host, project, session_id, intent, topic, source) = row?;
        by_host_key
            .entry((host, project, session_id))
            .or_insert((intent, topic, source));
    }

    let fallback_sql = format!(
        "SELECT memory_session_id, project, session_intent, session_topic, session_intent_source
         FROM session_summaries
         WHERE memory_session_id IN ({placeholders})
         ORDER BY COALESCE(session_intent_updated_at_epoch, created_at_epoch) DESC, id DESC"
    );
    let mut statement = conn.prepare(&fallback_sql)?;
    let rows = statement.query_map(params_from_iter(session_ids.iter()), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (memory_session_id, project, intent, topic, source) = row?;
        if let Some(project) = project {
            by_memory_key
                .entry((memory_session_id, project))
                .or_insert((intent, topic, source));
        }
    }
    Ok(())
}
