use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{params, Connection};

use super::{
    ensure_observation_retention_schema_supported, load_observation_retention_row,
    retention_record_from_row, OBSERVATION_RETENTION_SCHEMA_COLUMNS,
};

fn setup_observations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            type TEXT NOT NULL,
            title TEXT,
            subtitle TEXT,
            narrative TEXT,
            facts TEXT,
            concepts TEXT,
            files_read TEXT,
            files_modified TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER,
            branch TEXT,
            commit_sha TEXT,
            host_id INTEGER,
            project_id INTEGER,
            session_row_id INTEGER,
            observation_type TEXT,
            text TEXT,
            evidence_event_ids TEXT,
            confidence REAL,
            reference_time_epoch INTEGER
        );
        INSERT INTO observations (
            id, memory_session_id, project, type, title, subtitle, narrative,
            facts, concepts, files_read, files_modified, prompt_number,
            created_at, created_at_epoch, discovery_tokens, status,
            last_accessed_epoch, branch, commit_sha, host_id, project_id,
            session_row_id, observation_type, text, evidence_event_ids,
            confidence, reference_time_epoch
        ) VALUES (
            1, 'session-1', '/repo', 'decision', 'title', 'subtitle',
            'narrative', '[\"fact\"]', '[\"concept\"]', '[\"read\"]',
            '[\"modified\"]', 7, '2026-07-28T00:00:00Z', 1700000000, 42,
            'active', 1700000001, 'main', 'abc123', 11, 12, 13, 'decision',
            'evidence text', '[101]', 0.75, 1699999999
        );",
    )?;
    Ok(())
}

fn seeded_connection() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    setup_observations(&conn)?;
    Ok(conn)
}

fn record_for_id(
    conn: &Connection,
    observation_id: i64,
) -> Result<super::ObservationSourceRetentionRecord> {
    retention_record_from_row(load_observation_retention_row(conn, observation_id)?)
}

fn canonical_schema_columns() -> BTreeSet<String> {
    OBSERVATION_RETENTION_SCHEMA_COLUMNS
        .iter()
        .filter(|column| !matches!(**column, "status" | "last_accessed_epoch"))
        .map(|column| (*column).to_string())
        .collect()
}

#[test]
fn v2_snapshot_exactly_matches_the_canonical_schema_contract() -> Result<()> {
    let conn = seeded_connection()?;
    ensure_observation_retention_schema_supported(&conn)?;
    let record = record_for_id(&conn, 1)?;
    let snapshot = serde_json::from_str::<serde_json::Value>(&record.source_snapshot_json)?;

    assert_eq!(
        snapshot,
        serde_json::json!({
            "hash_version": "observation-v2",
            "id": 1,
            "memory_session_id": "session-1",
            "project": "/repo",
            "type": "decision",
            "title": "title",
            "subtitle": "subtitle",
            "narrative": "narrative",
            "facts": "[\"fact\"]",
            "concepts": "[\"concept\"]",
            "files_read": "[\"read\"]",
            "files_modified": "[\"modified\"]",
            "prompt_number": 7,
            "created_at": "2026-07-28T00:00:00Z",
            "created_at_epoch": 1700000000_i64,
            "discovery_tokens": 42,
            "branch": "main",
            "commit_sha": "abc123",
            "host_id": 11,
            "project_id": 12,
            "session_row_id": 13,
            "observation_type": "decision",
            "text": "evidence text",
            "evidence_event_ids": "[101]",
            "confidence": 0.75,
            "reference_time_epoch": 1699999999_i64,
        })
    );
    let actual_keys = snapshot
        .as_object()
        .expect("retention snapshot must be a JSON object")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut expected_keys = canonical_schema_columns();
    expected_keys.insert("hash_version".to_string());
    assert_eq!(actual_keys, expected_keys);
    Ok(())
}

#[test]
fn every_canonical_column_changes_the_v2_record() -> Result<()> {
    const MUTATIONS: &[(&str, &str, i64)] = &[
        ("id", "UPDATE observations SET id = 2 WHERE id = 1", 2),
        (
            "memory_session_id",
            "UPDATE observations SET memory_session_id = 'session-2' WHERE id = 1",
            1,
        ),
        (
            "project",
            "UPDATE observations SET project = '/other' WHERE id = 1",
            1,
        ),
        (
            "type",
            "UPDATE observations SET type = 'discovery' WHERE id = 1",
            1,
        ),
        (
            "title",
            "UPDATE observations SET title = 'other title' WHERE id = 1",
            1,
        ),
        (
            "subtitle",
            "UPDATE observations SET subtitle = 'other subtitle' WHERE id = 1",
            1,
        ),
        (
            "narrative",
            "UPDATE observations SET narrative = 'other narrative' WHERE id = 1",
            1,
        ),
        (
            "facts",
            "UPDATE observations SET facts = '[\"other fact\"]' WHERE id = 1",
            1,
        ),
        (
            "concepts",
            "UPDATE observations SET concepts = '[\"other concept\"]' WHERE id = 1",
            1,
        ),
        (
            "files_read",
            "UPDATE observations SET files_read = '[\"other read\"]' WHERE id = 1",
            1,
        ),
        (
            "files_modified",
            "UPDATE observations SET files_modified = '[\"other modified\"]' WHERE id = 1",
            1,
        ),
        (
            "prompt_number",
            "UPDATE observations SET prompt_number = 8 WHERE id = 1",
            1,
        ),
        (
            "created_at",
            "UPDATE observations SET created_at = '2026-07-29T00:00:00Z' WHERE id = 1",
            1,
        ),
        (
            "created_at_epoch",
            "UPDATE observations SET created_at_epoch = 1700000002 WHERE id = 1",
            1,
        ),
        (
            "discovery_tokens",
            "UPDATE observations SET discovery_tokens = 43 WHERE id = 1",
            1,
        ),
        (
            "branch",
            "UPDATE observations SET branch = 'other' WHERE id = 1",
            1,
        ),
        (
            "commit_sha",
            "UPDATE observations SET commit_sha = 'def456' WHERE id = 1",
            1,
        ),
        (
            "host_id",
            "UPDATE observations SET host_id = 21 WHERE id = 1",
            1,
        ),
        (
            "project_id",
            "UPDATE observations SET project_id = 22 WHERE id = 1",
            1,
        ),
        (
            "session_row_id",
            "UPDATE observations SET session_row_id = 23 WHERE id = 1",
            1,
        ),
        (
            "observation_type",
            "UPDATE observations SET observation_type = 'other' WHERE id = 1",
            1,
        ),
        (
            "text",
            "UPDATE observations SET text = 'other evidence' WHERE id = 1",
            1,
        ),
        (
            "evidence_event_ids",
            "UPDATE observations SET evidence_event_ids = '[102]' WHERE id = 1",
            1,
        ),
        (
            "confidence",
            "UPDATE observations SET confidence = 0.5 WHERE id = 1",
            1,
        ),
        (
            "reference_time_epoch",
            "UPDATE observations SET reference_time_epoch = 1699999998 WHERE id = 1",
            1,
        ),
    ];
    let mutation_columns = MUTATIONS
        .iter()
        .map(|(column, _, _)| (*column).to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(mutation_columns, canonical_schema_columns());

    for (column, mutation, result_id) in MUTATIONS {
        let conn = seeded_connection()?;
        let baseline = record_for_id(&conn, 1)?;
        conn.execute_batch(mutation)?;
        let changed = record_for_id(&conn, *result_id)?;
        assert_ne!(
            changed, baseline,
            "canonical column {column} must affect the v2 record"
        );
    }
    Ok(())
}

#[test]
fn lifecycle_and_access_metadata_do_not_change_the_v2_record() -> Result<()> {
    let conn = seeded_connection()?;
    let baseline = record_for_id(&conn, 1)?;
    conn.execute(
        "UPDATE observations
         SET status = 'compressed', last_accessed_epoch = ?1
         WHERE id = 1",
        params![1700000099_i64],
    )?;
    assert_eq!(record_for_id(&conn, 1)?, baseline);
    Ok(())
}

#[test]
fn non_finite_confidence_is_rejected_before_serialization() -> Result<()> {
    let conn = seeded_connection()?;
    conn.execute(
        "UPDATE observations SET confidence = ?1 WHERE id = 1",
        params![f64::INFINITY],
    )?;
    let error = record_for_id(&conn, 1).expect_err("infinite confidence must fail closed");
    assert!(
        error.to_string().contains("non-finite confidence"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn missing_schema_columns_fail_closed() -> Result<()> {
    let conn = seeded_connection()?;
    conn.execute_batch("ALTER TABLE observations DROP COLUMN reference_time_epoch;")?;
    let error = ensure_observation_retention_schema_supported(&conn)
        .expect_err("missing provenance columns must fail closed");
    assert!(
        error.to_string().contains("reference_time_epoch"),
        "{error:#}"
    );
    Ok(())
}
