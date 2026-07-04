use anyhow::Result;
use rusqlite::{params, Connection};

/// Hash-based deduplication: find exact duplicates within a time window.
/// Returns observation IDs that are exact duplicates of the given content hash.
pub fn find_hash_duplicates(
    conn: &Connection,
    project: &str,
    content_hash: &str,
    window_secs: i64,
) -> Result<Vec<i64>> {
    let cutoff = chrono::Utc::now().timestamp() - window_secs;

    let mut stmt = conn.prepare(
        "SELECT id, text, narrative, title, facts
         FROM observations
         WHERE project = ?1
           AND status = 'active'
           AND created_at_epoch > ?2
           AND (
             (text IS NOT NULL AND length(text) > 0)
             OR (narrative IS NOT NULL AND length(narrative) > 0)
             OR (title IS NOT NULL AND length(title) > 0)
             OR (facts IS NOT NULL AND length(facts) > 0)
           )",
    )?;

    let rows = stmt.query_map(params![project, cutoff], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut candidates = Vec::new();
    for row in rows {
        let (id, text, narrative, title, facts) = row?;
        let Some(obs_text) = canonical_observation_text(
            text.as_deref(),
            narrative.as_deref(),
            title.as_deref(),
            facts.as_deref(),
        ) else {
            continue;
        };
        let obs_hash = crate::db::content_identity_hash(obs_text.as_bytes());
        let legacy_obs_hash = crate::db::legacy_content_identity_hash(obs_text.as_bytes());
        if obs_hash == content_hash || legacy_obs_hash == content_hash {
            candidates.push(id);
        }
    }

    Ok(candidates)
}

pub(crate) fn canonical_observation_text(
    text: Option<&str>,
    narrative: Option<&str>,
    title: Option<&str>,
    facts: Option<&str>,
) -> Option<String> {
    [text, narrative, title]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| first_fact_text(facts))
}

fn first_fact_text(facts: Option<&str>) -> Option<String> {
    let facts = facts?.trim();
    if facts.is_empty() {
        return None;
    }
    match serde_json::from_str::<Vec<String>>(facts) {
        Ok(values) => values
            .into_iter()
            .map(|value| value.trim().to_string())
            .find(|value| !value.is_empty()),
        Err(_) => Some(facts.to_string()),
    }
}
