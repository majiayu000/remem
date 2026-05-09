use anyhow::{Context, Result};
use rusqlite::Connection;

const V2_BASELINE_SQL: &str = include_str!("../migrations/v2_001_baseline.sql");
const V2_SCHEMA_VERSION: i64 = 1;

/// Apply the v2 baseline migration to a fresh `~/.remem/v2.sqlite` connection.
/// Independent from v1 migrations: the v2 DB file never sees v001-v004.
pub(crate) fn run_v2_migrations(conn: &Connection) -> Result<()> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap_or(0);
    if current >= V2_SCHEMA_VERSION {
        return Ok(());
    }
    conn.execute_batch(V2_BASELINE_SQL)
        .context("v2_001_baseline migration failed")?;
    conn.execute_batch(&format!("PRAGMA user_version = {}", V2_SCHEMA_VERSION))?;
    crate::log::info("migrate", "applied v2_001_baseline");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_in_memory() -> Connection {
        Connection::open_in_memory().expect("open in-memory sqlite")
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count == 1
    }

    fn index_exists(conn: &Connection, name: &str) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap_or(0);
        count == 1
    }

    #[test]
    fn v2_baseline_creates_all_tables() {
        let conn = open_in_memory();
        run_v2_migrations(&conn).expect("v2 migration");
        for table in [
            "hosts",
            "workspaces",
            "projects",
            "sessions",
            "captured_events",
            "event_blobs",
            "extraction_tasks",
            "session_summaries",
            "observations",
            "memory_candidates",
            "memories",
            "rule_candidates",
            "worker_heartbeats",
        ] {
            assert!(table_exists(&conn, table), "table {} missing", table);
        }
    }

    #[test]
    fn v2_baseline_creates_memories_fts() {
        let conn = open_in_memory();
        run_v2_migrations(&conn).expect("v2 migration");
        // FTS5 virtual tables show up as 'table' rows in sqlite_master.
        assert!(table_exists(&conn, "memories_fts"));
    }

    #[test]
    fn v2_baseline_creates_critical_indexes() {
        let conn = open_in_memory();
        run_v2_migrations(&conn).expect("v2 migration");
        for idx in [
            "idx_sessions_host_project_seen",
            "idx_captured_events_session_event",
            "idx_extraction_tasks_claim",
            "idx_extraction_tasks_lease",
            "idx_memories_topic_unique",
        ] {
            assert!(index_exists(&conn, idx), "index {} missing", idx);
        }
    }

    #[test]
    fn v2_baseline_seeds_two_hosts() {
        let conn = open_in_memory();
        run_v2_migrations(&conn).expect("v2 migration");
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM hosts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
        let names: Vec<String> = conn
            .prepare("SELECT name FROM hosts ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(names, vec!["claude-code", "codex-cli"]);
    }

    #[test]
    fn v2_baseline_is_idempotent() {
        let conn = open_in_memory();
        run_v2_migrations(&conn).expect("first run");
        run_v2_migrations(&conn).expect("second run");
        let host_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM hosts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(host_count, 2, "seed must not duplicate on re-run");
    }

    #[test]
    fn v2_baseline_rejects_unique_topic_collision() {
        let conn = open_in_memory();
        run_v2_migrations(&conn).expect("v2 migration");
        // Insert two memories with same (scope, project_id=NULL, topic_key).
        // The expression-based unique index must reject the second one.
        conn.execute(
            "INSERT INTO memories(project_id, scope, memory_type, topic_key, text,
             evidence_event_ids, confidence, status, created_at_epoch, updated_at_epoch)
             VALUES (NULL, 'global', 'preference', 'k1', 't1', '[]', 0.9, 'active', 0, 0)",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO memories(project_id, scope, memory_type, topic_key, text,
             evidence_event_ids, confidence, status, created_at_epoch, updated_at_epoch)
             VALUES (NULL, 'global', 'preference', 'k1', 't2', '[]', 0.9, 'active', 0, 0)",
            [],
        );
        assert!(
            dup.is_err(),
            "expected UNIQUE violation on duplicate topic_key"
        );
    }
}
