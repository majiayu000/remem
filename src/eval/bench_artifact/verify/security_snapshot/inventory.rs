use anyhow::{ensure, Context, Result};
use rusqlite::Connection;

use crate::eval::memory_bench::types::MemoryBenchTask;

const BENCH_PROJECT: &str = "/tmp/remem-memory-bench/repo";
const TASK_BOUND_TABLES: &[&str] = &[
    "captured_events",
    "extraction_tasks",
    "entities",
    "hosts",
    "memories",
    "memory_activation_requests",
    "memory_candidates",
    "memory_edges",
    "memory_embeddings",
    "memory_entities",
    "memory_facts",
    "memory_operation_log",
    "memory_state_keys",
    "observations",
    "projects",
    "sessions",
    "workspaces",
];
const ALLOWED_METADATA_TABLES: &[&str] = &[
    "_schema_migrations",
    "legacy_surface_state",
    "retrieval_enrichment_compatibility",
    "sqlite_sequence",
    "memories_fts",
    "memories_fts_config",
    "memories_fts_data",
    "memories_fts_docsize",
    "memories_fts_idx",
    "observations_fts",
    "observations_fts_config",
    "observations_fts_data",
    "observations_fts_docsize",
    "observations_fts_idx",
    "raw_messages_fts",
    "raw_messages_fts_config",
    "raw_messages_fts_data",
    "raw_messages_fts_docsize",
    "raw_messages_fts_idx",
];

pub(super) fn validate_closed_world(
    connection: &Connection,
    task: &MemoryBenchTask,
    expected_event_count: usize,
) -> Result<()> {
    reject_unexpected_business_rows(connection)?;
    validate_identity_roots(connection, &task.id)?;
    validate_task_rows(connection, task, expected_event_count)?;
    validate_metadata(connection)?;
    Ok(())
}

fn reject_unexpected_business_rows(connection: &Connection) -> Result<()> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for table in tables {
        if TASK_BOUND_TABLES.contains(&table.as_str())
            || ALLOWED_METADATA_TABLES.contains(&table.as_str())
        {
            continue;
        }
        let count = table_row_count(connection, &table)?;
        ensure!(
            count == 0,
            "closed-world snapshot inventory found {count} row(s) in unrelated table {table}"
        );
    }
    Ok(())
}

fn table_row_count(connection: &Connection, table: &str) -> Result<i64> {
    ensure!(
        table
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "snapshot table name is not a safe SQLite identifier: {table:?}"
    );
    connection
        .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
            row.get(0)
        })
        .with_context(|| format!("count snapshot table {table}"))
}

fn validate_identity_roots(connection: &Connection, task_id: &str) -> Result<()> {
    let mut host_statement = connection.prepare("SELECT name FROM hosts ORDER BY name")?;
    let hosts = host_statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ensure!(
        hosts == ["claude-code", "codex-cli"],
        "closed-world snapshot inventory has unexpected host identities: {hosts:?}"
    );

    ensure!(
        table_row_count(connection, "workspaces")? == 1
            && connection.query_row(
                "SELECT COUNT(*) FROM workspaces WHERE root_path = ?1",
                [BENCH_PROJECT],
                |row| row.get::<_, i64>(0),
            )? == 1,
        "closed-world snapshot inventory must contain exactly the benchmark workspace"
    );
    ensure!(
        table_row_count(connection, "projects")? == 1
            && connection.query_row(
                "SELECT COUNT(*)
                 FROM projects p
                 JOIN workspaces w ON w.id = p.workspace_id
                 WHERE p.project_path = ?1 AND w.root_path = ?1",
                [BENCH_PROJECT],
                |row| row.get::<_, i64>(0),
            )? == 1,
        "closed-world snapshot inventory must contain exactly the benchmark project"
    );
    ensure!(
        table_row_count(connection, "sessions")? == 1
            && connection.query_row(
                "SELECT COUNT(*)
                 FROM sessions s
                 JOIN hosts h ON h.id = s.host_id
                 JOIN projects p ON p.id = s.project_id
                 JOIN workspaces w ON w.id = s.workspace_id
                 WHERE s.session_id = ?1 AND h.name = 'codex-cli'
                   AND p.project_path = ?2 AND w.root_path = ?2",
                rusqlite::params![task_id, BENCH_PROJECT],
                |row| row.get::<_, i64>(0),
            )? == 1,
        "closed-world snapshot inventory must contain exactly the declared benchmark session"
    );
    Ok(())
}

fn validate_task_rows(
    connection: &Connection,
    task: &MemoryBenchTask,
    expected_event_count: usize,
) -> Result<()> {
    let policy = task.policy.as_ref().context("security task lacks policy")?;
    let expected_observations =
        usize::from(policy.explicit_approval || policy.poisoning_quarantine_expected);
    let expected_memories = policy.expected_active_claims as usize;
    let expected_candidates = expected_memories + policy.expected_candidates as usize;
    let expected_tasks = 1 + expected_observations + expected_candidates;

    ensure!(
        table_row_count(connection, "captured_events")? == expected_event_count as i64,
        "closed-world snapshot inventory captured_events count differs from the typed suite"
    );
    ensure_task_scoped_count(
        connection,
        "observations",
        expected_observations,
        "SELECT COUNT(*)
         FROM observations o
         JOIN sessions s ON s.id = o.session_row_id
         JOIN projects p ON p.id = o.project_id
         WHERE s.session_id = ?1 AND p.project_path = ?2",
        &task.id,
    )?;
    for table in [
        "entities",
        "memory_activation_requests",
        "memory_edges",
        "memory_embeddings",
        "memory_entities",
        "memory_facts",
        "memory_operation_log",
        "memory_state_keys",
    ] {
        ensure!(
            table_row_count(connection, table)? == expected_memories as i64,
            "closed-world snapshot inventory for {table} must contain exactly {expected_memories} row(s) derived from declared active memory"
        );
    }
    ensure_task_scoped_count(
        connection,
        "memory_candidates",
        expected_candidates,
        "SELECT COUNT(*)
         FROM memory_candidates c
         JOIN projects p ON p.id = c.project_id
         WHERE p.project_path = ?2
           AND NOT EXISTS (
             SELECT 1 FROM json_each(c.evidence_event_ids) evidence
             LEFT JOIN captured_events e ON e.id = evidence.value
             WHERE e.id IS NULL OR e.session_id != ?1
           )",
        &task.id,
    )?;
    ensure_task_scoped_count(
        connection,
        "memories",
        expected_memories,
        "SELECT COUNT(*)
         FROM memories m
         WHERE m.session_id = ?1 AND m.project = ?2
           AND m.status = 'active'
           AND m.source_candidate_id IN (SELECT id FROM memory_candidates)",
        &task.id,
    )?;
    ensure_task_scoped_count(
        connection,
        "extraction_tasks",
        expected_tasks,
        "SELECT COUNT(*)
         FROM extraction_tasks t
         JOIN sessions s ON s.id = t.session_row_id
         JOIN projects p ON p.id = t.project_id
         WHERE s.session_id = ?1 AND p.project_path = ?2
           AND t.last_error IS NULL
           AND t.task_kind IN ('observation_extract', 'memory_candidate', 'graph_candidate')",
        &task.id,
    )?;
    Ok(())
}

fn ensure_task_scoped_count(
    connection: &Connection,
    table: &str,
    expected: usize,
    scoped_query: &str,
    task_id: &str,
) -> Result<()> {
    let total = table_row_count(connection, table)?;
    let scoped = connection.query_row(
        scoped_query,
        rusqlite::params![task_id, BENCH_PROJECT],
        |row| row.get::<_, i64>(0),
    )?;
    ensure!(
        total == expected as i64 && scoped == total,
        "closed-world snapshot inventory for {table} expected {expected} task-bound row(s), found total={total} task_bound={scoped}"
    );
    Ok(())
}

fn validate_metadata(connection: &Connection) -> Result<()> {
    let legacy: (String, String, i64) = connection.query_row(
        "SELECT surface, state, residual_count FROM legacy_surface_state",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    ensure!(
        table_row_count(connection, "legacy_surface_state")? == 1
            && legacy
                == (
                    "pending_observations".to_string(),
                    "exhausted".to_string(),
                    0
                ),
        "closed-world snapshot inventory has unexpected legacy migration state"
    );
    let compatibility: (i64, i64, i64, String) = connection.query_row(
        "SELECT min_security_policy_version, compatibility_epoch,
                target_security_policy_version, convergence_state
         FROM retrieval_enrichment_compatibility",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    ensure!(
        table_row_count(connection, "retrieval_enrichment_compatibility")? == 1
            && compatibility == (1, 1, 1, "ready".to_string()),
        "closed-world snapshot inventory has unexpected compatibility metadata"
    );
    Ok(())
}
