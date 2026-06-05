use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::db::models::{CompressedObservationSource, Observation};

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
        "UPDATE observations
         SET status = 'compressed'
         WHERE status IN ('active', 'stale') AND id IN ({})",
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

pub fn observation_source_hash(observation: &Observation) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, "hash_version", Some("observation-v1"));
    hash_i64(&mut hasher, "id", observation.id);
    hash_field(
        &mut hasher,
        "memory_session_id",
        Some(observation.memory_session_id.as_str()),
    );
    hash_field(&mut hasher, "project", observation.project.as_deref());
    hash_field(&mut hasher, "type", Some(observation.r#type.as_str()));
    hash_field(&mut hasher, "title", observation.title.as_deref());
    hash_field(&mut hasher, "subtitle", observation.subtitle.as_deref());
    hash_field(&mut hasher, "narrative", observation.narrative.as_deref());
    hash_field(&mut hasher, "facts", observation.facts.as_deref());
    hash_field(&mut hasher, "concepts", observation.concepts.as_deref());
    hash_field(&mut hasher, "files_read", observation.files_read.as_deref());
    hash_field(
        &mut hasher,
        "files_modified",
        observation.files_modified.as_deref(),
    );
    hash_optional_i64(
        &mut hasher,
        "discovery_tokens",
        observation.discovery_tokens,
    );
    hash_field(
        &mut hasher,
        "created_at",
        Some(observation.created_at.as_str()),
    );
    hash_i64(
        &mut hasher,
        "created_at_epoch",
        observation.created_at_epoch,
    );
    hash_field(
        &mut hasher,
        "content_session_id",
        observation.content_session_id.as_deref(),
    );
    hash_field(&mut hasher, "branch", observation.branch.as_deref());
    hash_field(&mut hasher, "commit_sha", observation.commit_sha.as_deref());

    format!("sha256:observation-v1:{:x}", hasher.finalize())
}

pub fn insert_compressed_observation_sources(
    conn: &Connection,
    compressed_observation_ids: &[i64],
    source_observations: &[Observation],
    compression_session_id: &str,
) -> Result<usize> {
    if compressed_observation_ids.is_empty() {
        return Ok(0);
    }
    if source_observations.is_empty() {
        anyhow::bail!("compressed observations require source observation links");
    }

    let now = chrono::Utc::now().timestamp();
    let mut inserted = 0;
    for compressed_id in compressed_observation_ids {
        for source in source_observations {
            inserted += conn.execute(
                "INSERT INTO compressed_observation_sources
                 (compressed_observation_id, source_observation_id, source_hash,
                  source_snapshot_json, source_created_at_epoch, compression_session_id,
                  created_at_epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    *compressed_id,
                    source.id,
                    observation_source_hash(source),
                    observation_source_snapshot_json(source)?,
                    source.created_at_epoch,
                    compression_session_id,
                    now
                ],
            )?;
        }
    }
    Ok(inserted)
}

pub fn load_compressed_observation_sources(
    conn: &Connection,
    compressed_observation_ids: &[i64],
) -> Result<HashMap<i64, Vec<CompressedObservationSource>>> {
    if compressed_observation_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders: Vec<String> = (1..=compressed_observation_ids.len())
        .map(|i| format!("?{i}"))
        .collect();
    let sql = format!(
        "SELECT compressed_observation_id, source_observation_id, source_hash,
                source_snapshot_json, source_created_at_epoch, compression_session_id,
                created_at_epoch
         FROM compressed_observation_sources
         WHERE compressed_observation_id IN ({})
         ORDER BY compressed_observation_id, source_observation_id",
        placeholders.join(", ")
    );
    let params: Vec<Box<dyn rusqlite::types::ToSql>> = compressed_observation_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let refs = super::core::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(CompressedObservationSource {
            compressed_observation_id: row.get(0)?,
            source_observation_id: row.get(1)?,
            source_hash: row.get(2)?,
            source_snapshot_json: row.get(3)?,
            source_created_at_epoch: row.get(4)?,
            compression_session_id: row.get(5)?,
            created_at_epoch: row.get(6)?,
        })
    })?;

    let mut by_compressed_id: HashMap<i64, Vec<CompressedObservationSource>> = HashMap::new();
    for row in rows {
        let source = row?;
        by_compressed_id
            .entry(source.compressed_observation_id)
            .or_default()
            .push(source);
    }
    Ok(by_compressed_id)
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

fn hash_i64(hasher: &mut Sha256, name: &str, value: i64) {
    let value = value.to_string();
    hash_field(hasher, name, Some(value.as_str()));
}

fn observation_source_snapshot_json(observation: &Observation) -> Result<String> {
    let snapshot = serde_json::json!({
        "hash_version": "observation-v1",
        "id": observation.id,
        "memory_session_id": observation.memory_session_id.as_str(),
        "project": observation.project.as_deref(),
        "type": observation.r#type.as_str(),
        "title": observation.title.as_deref(),
        "subtitle": observation.subtitle.as_deref(),
        "narrative": observation.narrative.as_deref(),
        "facts": observation.facts.as_deref(),
        "concepts": observation.concepts.as_deref(),
        "files_read": observation.files_read.as_deref(),
        "files_modified": observation.files_modified.as_deref(),
        "discovery_tokens": observation.discovery_tokens,
        "created_at": observation.created_at.as_str(),
        "created_at_epoch": observation.created_at_epoch,
        "content_session_id": observation.content_session_id.as_deref(),
        "branch": observation.branch.as_deref(),
        "commit_sha": observation.commit_sha.as_deref(),
    });
    Ok(serde_json::to_string(&snapshot)?)
}

fn hash_optional_i64(hasher: &mut Sha256, name: &str, value: Option<i64>) {
    match value {
        Some(value) => hash_i64(hasher, name, value),
        None => hash_field(hasher, name, None),
    }
}

fn hash_field(hasher: &mut Sha256, name: &str, value: Option<&str>) {
    hasher.update(name.as_bytes());
    hasher.update(b"\x1f");
    match value {
        Some(value) => {
            hasher.update(value.len().to_string().as_bytes());
            hasher.update(b"\x1e");
            hasher.update(value.as_bytes());
        }
        None => hasher.update(b"-1"),
    }
    hasher.update(b"\x1d");
}
