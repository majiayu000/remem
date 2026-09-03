use std::collections::HashSet;

use anyhow::Result;
use rusqlite::params;

use crate::context::injection_gate::injection_key_for_audit;
use crate::context::invocation::ContextInvocation;

pub(super) fn prompt_injection_key(
    invocation: &ContextInvocation,
    prompt_event_id: Option<&str>,
) -> Option<String> {
    prompt_event_id.map(|event_id| {
        format!(
            "{}:prompt-event:{event_id}",
            injection_key_for_audit(invocation)
        )
    })
}

pub(super) fn previously_injected_memory_ids(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    current_prompt_key: Option<&str>,
) -> Result<HashSet<i64>> {
    let key = injection_key_for_audit(invocation);
    let prompt_key_prefix = format!("{key}:prompt-event:");
    let Some(session_id) = invocation.session_id.as_deref() else {
        return Ok(HashSet::new());
    };
    let mut stmt = conn.prepare(
        "SELECT DISTINCT memory_id
         FROM context_injection_items
         WHERE host = ?1
           AND project = ?2
           AND session_id = ?3
           AND (injection_key = ?4
                OR substr(injection_key, 1, length(?5)) = ?5)
           AND (?6 IS NULL OR injection_key != ?6)
           AND status = 'injected'
           AND memory_id IS NOT NULL",
    )?;
    let rows = stmt.query_map(
        params![
            invocation.host.as_env_value(),
            invocation.project,
            session_id,
            key,
            prompt_key_prefix,
            current_prompt_key,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(crate::db::query::collect_rows(rows)?.into_iter().collect())
}
