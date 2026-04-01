use anyhow::Result;
use rusqlite::{params, Connection};

pub fn insert_observation(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
    obs_type: &str,
    title: Option<&str>,
    subtitle: Option<&str>,
    narrative: Option<&str>,
    facts: Option<&str>,
    concepts: Option<&str>,
    files_read: Option<&str>,
    files_modified: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
) -> Result<i64> {
    insert_observation_with_branch(
        conn,
        memory_session_id,
        project,
        obs_type,
        title,
        subtitle,
        narrative,
        facts,
        concepts,
        files_read,
        files_modified,
        prompt_number,
        discovery_tokens,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn insert_observation_with_branch(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
    obs_type: &str,
    title: Option<&str>,
    subtitle: Option<&str>,
    narrative: Option<&str>,
    facts: Option<&str>,
    concepts: Option<&str>,
    files_read: Option<&str>,
    files_modified: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
    branch: Option<&str>,
    commit_sha: Option<&str>,
) -> Result<i64> {
    let now = chrono::Utc::now();
    conn.execute(
        "INSERT INTO observations \
         (memory_session_id, project, type, title, subtitle, narrative, \
          facts, concepts, files_read, files_modified, prompt_number, \
          created_at, created_at_epoch, discovery_tokens, branch, commit_sha) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            memory_session_id,
            project,
            obs_type,
            title,
            subtitle,
            narrative,
            facts,
            concepts,
            files_read,
            files_modified,
            prompt_number,
            now.to_rfc3339(),
            now.timestamp(),
            discovery_tokens,
            branch,
            commit_sha
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn mark_stale_by_files(
    conn: &Connection,
    new_obs_id: i64,
    project: &str,
    files_modified: &[String],
) -> Result<usize> {
    if files_modified.is_empty() {
        return Ok(0);
    }
    let files_json = serde_json::to_string(files_modified)?;
    let count = conn.execute(
        "UPDATE observations SET status = 'stale'
         WHERE id != ?1 AND project = ?2 AND status = 'active'
           AND id IN (
             SELECT DISTINCT o.id FROM observations o, json_each(o.files_modified) AS old_f
             WHERE o.id != ?1 AND o.project = ?2 AND o.status = 'active'
               AND o.files_modified IS NOT NULL AND length(o.files_modified) > 2
               AND old_f.value IN (SELECT value FROM json_each(?3))
           )",
        params![new_obs_id, project, files_json],
    )?;
    Ok(count)
}

pub fn mark_observations_compressed(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE observations SET status = 'compressed' WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let refs = super::core::to_sql_refs(&params);
    Ok(stmt.execute(refs.as_slice())?)
}

pub fn update_last_accessed(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    let placeholders: Vec<String> = (2..=ids.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE observations SET last_accessed_epoch = ?1 WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
    for id in ids {
        params.push(Box::new(*id));
    }
    let refs = super::core::to_sql_refs(&params);
    stmt.execute(refs.as_slice())?;
    Ok(())
}
