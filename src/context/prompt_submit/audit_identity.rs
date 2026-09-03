use std::collections::HashSet;

use crate::context::injection_gate::injection_key_for_audit;
use crate::context::invocation::ContextInvocation;
use anyhow::Result;

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
    let Some(session_id) = invocation.session_id.as_deref() else {
        return Ok(HashSet::new());
    };
    let projects = crate::project_alias::project_filter_values(conn, &invocation.project)?;
    let mut keys = Vec::with_capacity(projects.len());
    let mut prompt_key_prefixes = Vec::with_capacity(projects.len());
    for project in &projects {
        let mut scoped_invocation = invocation.clone();
        scoped_invocation.project = project.clone();
        let key = injection_key_for_audit(&scoped_invocation);
        prompt_key_prefixes.push(format!("{key}:prompt-event:"));
        keys.push(key);
    }
    let current_event = current_prompt_key
        .and_then(|key| key.split_once(":prompt-event:"))
        .map(|(_, event_id)| event_id);
    let current_prompt_keys = current_event
        .map(|event_id| {
            prompt_key_prefixes
                .iter()
                .map(|prefix| format!("{prefix}{event_id}"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let projects_json = serde_json::to_string(&projects)?;
    let keys_json = serde_json::to_string(&keys)?;
    let prompt_prefixes_json = serde_json::to_string(&prompt_key_prefixes)?;
    let current_keys_json = serde_json::to_string(&current_prompt_keys)?;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT memory_id
         FROM context_injection_items
         WHERE host = ?1
           AND project IN (SELECT value FROM json_each(?2))
           AND session_id = ?3
           AND (injection_key IN (SELECT value FROM json_each(?4))
                OR EXISTS (
                    SELECT 1 FROM json_each(?5) prefixes
                    WHERE substr(context_injection_items.injection_key, 1, length(prefixes.value))
                          = prefixes.value
                ))
           AND injection_key NOT IN (SELECT value FROM json_each(?6))
           AND status = 'injected'
           AND memory_id IS NOT NULL",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![
            invocation.host.as_env_value(),
            projects_json,
            session_id,
            keys_json,
            prompt_prefixes_json,
            current_keys_json,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(crate::db::query::collect_rows(rows)?.into_iter().collect())
}
