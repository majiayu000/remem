use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::poisoning::scan_generated_surfaces;
use crate::memory::session_label::SessionIntentSource;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionIntentWrite {
    pub session_intent: Option<String>,
    pub session_topic: Option<String>,
}

impl SessionIntentWrite {
    fn persist_columns(
        &self,
        created_at_epoch: i64,
    ) -> (
        Option<&str>,
        Option<&str>,
        Option<&'static str>,
        Option<i64>,
    ) {
        let intent = self.session_intent.as_deref();
        let topic = self.session_topic.as_deref();
        if intent.is_none() && topic.is_none() {
            return (None, None, None, None);
        }
        (
            intent,
            topic,
            Some(SessionIntentSource::Summary.as_str()),
            Some(created_at_epoch),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_summarize(
    conn: &mut Connection,
    memory_session_id: &str,
    project: &str,
    message_hash: &str,
    request: Option<&str>,
    completed: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    next_steps: Option<&str>,
    preferences: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
    session_intent: SessionIntentWrite,
) -> Result<usize> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();
    let (session_intent, session_topic, session_intent_source, session_intent_updated_at_epoch) =
        session_intent.persist_columns(created_at_epoch);
    let verdict = scan_generated_surfaces(&[
        ("request", request),
        ("completed", completed),
        ("decisions", decisions),
        ("learned", learned),
        ("next_steps", next_steps),
        ("preferences", preferences),
    ]);
    if let Some(surface_match) = &verdict {
        crate::log::error(
            "summarize",
            &format!(
                "quarantining finalized summary for session {memory_session_id}: field={} pattern={}@v{}",
                surface_match.field,
                surface_match.pattern.pattern_id,
                surface_match.pattern.pattern_set_version,
            ),
        );
    }

    let tx = conn.transaction()?;
    let deleted = tx.execute(
        "DELETE FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
        params![memory_session_id, project],
    )?;
    tx.execute(
        "INSERT INTO session_summaries \
         (memory_session_id, project, request, completed, decisions, learned, \
          next_steps, preferences, prompt_number, created_at, created_at_epoch, \
          discovery_tokens, poisoning_status, quarantine_stage, quarantine_field, \
          quarantine_pattern_id, quarantine_pattern_version, session_intent, \
          session_topic, session_intent_source, session_intent_updated_at_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        params![
            memory_session_id,
            project,
            request,
            completed,
            decisions,
            learned,
            next_steps,
            preferences,
            prompt_number,
            created_at,
            created_at_epoch,
            discovery_tokens,
            if verdict.is_some() {
                "quarantined"
            } else {
                "safe"
            },
            verdict.as_ref().map(|matched| matched.stage.as_str()),
            verdict.as_ref().map(|matched| matched.field.as_str()),
            verdict.as_ref().map(|matched| matched.pattern.pattern_id),
            verdict
                .as_ref()
                .map(|matched| matched.pattern.pattern_set_version),
            session_intent,
            session_topic,
            session_intent_source,
            session_intent_updated_at_epoch,
        ],
    )?;
    tx.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?2,
           last_message_hash = ?3",
        params![project, created_at_epoch, message_hash],
    )?;
    tx.commit()?;
    Ok(deleted)
}
