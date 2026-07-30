use std::collections::BTreeSet;

use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::db::models::Observation;

#[cfg(test)]
mod tests;

pub(crate) const OBSERVATION_RETENTION_SCHEMA_COLUMNS: &[&str] = &[
    "id",
    "memory_session_id",
    "project",
    "type",
    "title",
    "subtitle",
    "narrative",
    "facts",
    "concepts",
    "files_read",
    "files_modified",
    "prompt_number",
    "created_at",
    "created_at_epoch",
    "discovery_tokens",
    "status",
    "last_accessed_epoch",
    "branch",
    "commit_sha",
    "host_id",
    "project_id",
    "session_row_id",
    "observation_type",
    "text",
    "evidence_event_ids",
    "confidence",
    "reference_time_epoch",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservationSourceRetentionRecord {
    pub source_hash: String,
    pub source_snapshot_json: String,
}

#[derive(Debug)]
struct ObservationRetentionRow {
    id: i64,
    memory_session_id: String,
    project: Option<String>,
    observation_kind: String,
    title: Option<String>,
    subtitle: Option<String>,
    narrative: Option<String>,
    facts: Option<String>,
    concepts: Option<String>,
    files_read: Option<String>,
    files_modified: Option<String>,
    prompt_number: Option<i64>,
    created_at: String,
    created_at_epoch: i64,
    discovery_tokens: Option<i64>,
    status: String,
    branch: Option<String>,
    commit_sha: Option<String>,
    host_id: Option<i64>,
    project_id: Option<i64>,
    session_row_id: Option<i64>,
    observation_type: Option<String>,
    text: Option<String>,
    evidence_event_ids: Option<String>,
    confidence: Option<f64>,
    reference_time_epoch: Option<i64>,
}

pub(crate) fn ensure_observation_retention_schema_supported(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info('observations')")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let actual = crate::db::query::collect_rows(rows)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let expected = OBSERVATION_RETENTION_SCHEMA_COLUMNS
        .iter()
        .map(|column| (*column).to_string())
        .collect::<BTreeSet<_>>();
    let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
    let unknown = actual.difference(&expected).cloned().collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        bail!(
            "unsupported observations retention schema: missing=[{}] unknown=[{}]",
            missing.join(","),
            unknown.join(",")
        );
    }
    Ok(())
}

pub(crate) fn observation_source_retention_records(
    conn: &Connection,
    observations: &[Observation],
) -> Result<Vec<ObservationSourceRetentionRecord>> {
    ensure_observation_retention_schema_supported(conn)?;
    observations
        .iter()
        .map(|observation| {
            observation_source_retention_record_on_supported_schema(conn, observation)
        })
        .collect()
}

#[cfg(test)]
pub(crate) fn observation_source_retention_record(
    conn: &Connection,
    observation: &Observation,
) -> Result<ObservationSourceRetentionRecord> {
    ensure_observation_retention_schema_supported(conn)?;
    observation_source_retention_record_on_supported_schema(conn, observation)
}

pub(crate) fn observation_source_retention_record_on_supported_schema(
    conn: &Connection,
    observation: &Observation,
) -> Result<ObservationSourceRetentionRecord> {
    let row = load_observation_retention_row(conn, observation.id)?;
    validate_observation_input(&row, observation)?;
    retention_record_from_row(row)
}

fn retention_record_from_row(
    row: ObservationRetentionRow,
) -> Result<ObservationSourceRetentionRecord> {
    if row.confidence.is_some_and(|value| !value.is_finite()) {
        bail!(
            "observation {} has non-finite confidence and cannot be snapshotted",
            row.id
        );
    }
    let snapshot = serde_json::json!({
        "hash_version": "observation-v2",
        "id": row.id,
        "memory_session_id": row.memory_session_id,
        "project": row.project,
        "type": row.observation_kind,
        "title": row.title,
        "subtitle": row.subtitle,
        "narrative": row.narrative,
        "facts": row.facts,
        "concepts": row.concepts,
        "files_read": row.files_read,
        "files_modified": row.files_modified,
        "prompt_number": row.prompt_number,
        "discovery_tokens": row.discovery_tokens,
        "created_at": row.created_at,
        "created_at_epoch": row.created_at_epoch,
        "branch": row.branch,
        "commit_sha": row.commit_sha,
        "host_id": row.host_id,
        "project_id": row.project_id,
        "session_row_id": row.session_row_id,
        "observation_type": row.observation_type,
        "text": row.text,
        "evidence_event_ids": row.evidence_event_ids,
        "confidence": row.confidence,
        "reference_time_epoch": row.reference_time_epoch,
    });
    let source_snapshot_json = serde_json::to_string(&snapshot)?;
    let digest = Sha256::digest(source_snapshot_json.as_bytes());
    Ok(ObservationSourceRetentionRecord {
        source_hash: format!("sha256:observation-v2:{digest:x}"),
        source_snapshot_json,
    })
}

fn load_observation_retention_row(
    conn: &Connection,
    observation_id: i64,
) -> Result<ObservationRetentionRow> {
    conn.query_row(
        "SELECT id, memory_session_id, project, type, title, subtitle, narrative,
                facts, concepts, files_read, files_modified, prompt_number,
                created_at, created_at_epoch, discovery_tokens,
                COALESCE(status, 'active'), branch, commit_sha, host_id,
                project_id, session_row_id, observation_type, text,
                evidence_event_ids, confidence, reference_time_epoch
         FROM observations
         WHERE id = ?1",
        params![observation_id],
        |row| {
            Ok(ObservationRetentionRow {
                id: row.get(0)?,
                memory_session_id: row.get(1)?,
                project: row.get(2)?,
                observation_kind: row.get(3)?,
                title: row.get(4)?,
                subtitle: row.get(5)?,
                narrative: row.get(6)?,
                facts: row.get(7)?,
                concepts: row.get(8)?,
                files_read: row.get(9)?,
                files_modified: row.get(10)?,
                prompt_number: row.get(11)?,
                created_at: row.get(12)?,
                created_at_epoch: row.get(13)?,
                discovery_tokens: row.get(14)?,
                status: row.get(15)?,
                branch: row.get(16)?,
                commit_sha: row.get(17)?,
                host_id: row.get(18)?,
                project_id: row.get(19)?,
                session_row_id: row.get(20)?,
                observation_type: row.get(21)?,
                text: row.get(22)?,
                evidence_event_ids: row.get(23)?,
                confidence: row.get(24)?,
                reference_time_epoch: row.get(25)?,
            })
        },
    )
    .with_context(|| format!("load observation {observation_id} retention provenance"))
}

fn validate_observation_input(
    row: &ObservationRetentionRow,
    observation: &Observation,
) -> Result<()> {
    ensure!(
        row.id == observation.id
            && row.memory_session_id == observation.memory_session_id
            && row.project == observation.project
            && row.observation_kind == observation.r#type
            && row.title == observation.title
            && row.subtitle == observation.subtitle
            && row.narrative == observation.narrative
            && row.facts == observation.facts
            && row.concepts == observation.concepts
            && row.files_read == observation.files_read
            && row.files_modified == observation.files_modified
            && row.created_at == observation.created_at
            && row.created_at_epoch == observation.created_at_epoch
            && row.discovery_tokens == observation.discovery_tokens
            && row.status == observation.status
            && row.branch == observation.branch
            && row.commit_sha == observation.commit_sha,
        "observation {} changed after compression input selection",
        observation.id
    );
    Ok(())
}
