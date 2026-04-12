use anyhow::Result;
use rusqlite::Connection;

use super::state::{has_migration_table, mark_applied};
use super::types::OLD_BASELINE_VERSION;

pub(super) fn transition_from_old_system(conn: &Connection) -> Result<()> {
    if has_existing_migration_entries(conn) {
        // Always run backfill even when migrations are already recorded.
        // backfill is idempotent (ADD COLUMN IF missing, CREATE IF NOT EXISTS)
        // and catches columns added to the baseline SQL after initial migration.
        backfill_to_baseline(conn)?;
        return Ok(());
    }

    let old_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if old_version >= OLD_BASELINE_VERSION {
        crate::log::info(
            "migrate",
            &format!(
                "transitioning from user_version={} to _schema_migrations",
                old_version
            ),
        );
        backfill_to_baseline(conn)?;
        mark_applied(conn, 1, "baseline")?;
    } else if old_version > 0 {
        crate::log::info(
            "migrate",
            &format!(
                "auto-upgrading schema from v{} to v{} baseline",
                old_version, OLD_BASELINE_VERSION
            ),
        );
        backfill_to_baseline(conn)?;
        mark_applied(conn, 1, "baseline")?;
    }

    Ok(())
}

/// Bring a pre-v13 database up to baseline by adding missing columns, tables,
/// and indexes. Uses IF NOT EXISTS / ignores "duplicate column" errors so it is
/// safe to run on any v1-v12 schema.
fn backfill_to_baseline(conn: &Connection) -> Result<()> {
    // --- missing columns on pending_observations (added between v10-v13) ---
    let pending_cols = [
        ("updated_at_epoch", "INTEGER NOT NULL DEFAULT 0"),
        ("status", "TEXT NOT NULL DEFAULT 'pending'"),
        ("attempt_count", "INTEGER NOT NULL DEFAULT 0"),
        ("next_retry_epoch", "INTEGER"),
        ("last_error", "TEXT"),
    ];
    for (col, typedef) in &pending_cols {
        add_column_if_missing(conn, "pending_observations", col, typedef);
    }

    // --- missing columns on observations ---
    let obs_cols = [
        ("discovery_tokens", "INTEGER DEFAULT 0"),
        ("status", "TEXT DEFAULT 'active'"),
        ("last_accessed_epoch", "INTEGER"),
        ("branch", "TEXT"),
        ("commit_sha", "TEXT"),
    ];
    for (col, typedef) in &obs_cols {
        add_column_if_missing(conn, "observations", col, typedef);
    }

    // --- missing columns on memories ---
    let mem_cols = [("branch", "TEXT"), ("scope", "TEXT DEFAULT 'project'")];
    for (col, typedef) in &mem_cols {
        add_column_if_missing(conn, "memories", col, typedef);
    }

    // --- missing columns on session_summaries ---
    let ss_cols = [("discovery_tokens", "INTEGER DEFAULT 0")];
    for (col, typedef) in &ss_cols {
        add_column_if_missing(conn, "session_summaries", col, typedef);
    }

    // --- tables that may not exist in older schemas ---
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY,
            canonical_name TEXT NOT NULL COLLATE NOCASE,
            entity_type TEXT,
            mention_count INTEGER DEFAULT 1,
            created_at_epoch INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            UNIQUE(canonical_name)
        );
        CREATE TABLE IF NOT EXISTS memory_entities (
            memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
            entity_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
            PRIMARY KEY(memory_id, entity_id)
        );
        CREATE TABLE IF NOT EXISTS workstreams (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL DEFAULT 'active',
            progress TEXT,
            next_action TEXT,
            blockers TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            completed_at_epoch INTEGER
        );
        CREATE TABLE IF NOT EXISTS workstream_sessions (
            id INTEGER PRIMARY KEY,
            workstream_id INTEGER NOT NULL,
            memory_session_id TEXT NOT NULL,
            linked_at_epoch INTEGER NOT NULL,
            UNIQUE(workstream_id, memory_session_id)
        );
        CREATE TABLE IF NOT EXISTS jobs (
            id INTEGER PRIMARY KEY,
            job_type TEXT NOT NULL,
            project TEXT NOT NULL,
            session_id TEXT,
            payload_json TEXT NOT NULL,
            state TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 100,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 6,
            lease_owner TEXT,
            lease_expires_epoch INTEGER,
            next_retry_epoch INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS ai_usage_events (
            id INTEGER PRIMARY KEY,
            created_at TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            project TEXT,
            operation TEXT NOT NULL,
            executor TEXT NOT NULL,
            model TEXT,
            input_tokens INTEGER NOT NULL,
            output_tokens INTEGER NOT NULL,
            total_tokens INTEGER NOT NULL,
            estimated_cost_usd REAL NOT NULL
        );",
    )?;

    // --- indexes (all IF NOT EXISTS, safe to re-run) ---
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_observations_status ON observations(status);
        CREATE INDEX IF NOT EXISTS idx_observations_project_status ON observations(project, status, created_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_observations_branch ON observations(project, branch, created_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_pending_session_lease ON pending_observations(session_id, lease_expires_epoch, id);
        CREATE INDEX IF NOT EXISTS idx_pending_project_lease ON pending_observations(project, lease_expires_epoch, created_at_epoch);
        CREATE INDEX IF NOT EXISTS idx_pending_claim_v2 ON pending_observations(status, session_id, next_retry_epoch, lease_expires_epoch, id);
        CREATE INDEX IF NOT EXISTS idx_pending_project_v2 ON pending_observations(status, project, next_retry_epoch, created_at_epoch);
        CREATE INDEX IF NOT EXISTS idx_sdk_sessions_msid ON sdk_sessions(memory_session_id);
        CREATE INDEX IF NOT EXISTS idx_memories_project_status ON memories(project, status, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_memories_topic_key ON memories(project, topic_key) WHERE topic_key IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(project, memory_type, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_memories_branch ON memories(project, branch, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope, status, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id, created_at_epoch);
        CREATE INDEX IF NOT EXISTS idx_events_project ON events(project, created_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_ai_usage_created ON ai_usage_events(created_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_ai_usage_project_created ON ai_usage_events(project, created_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_jobs_claim ON jobs(state, next_retry_epoch, priority, created_at_epoch, id);
        CREATE INDEX IF NOT EXISTS idx_jobs_project_state ON jobs(project, state, created_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_jobs_lease ON jobs(state, lease_expires_epoch);
        CREATE INDEX IF NOT EXISTS idx_workstreams_project_status ON workstreams(project, status, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_workstream_sessions_ws ON workstream_sessions(workstream_id);
        CREATE INDEX IF NOT EXISTS idx_workstream_sessions_session ON workstream_sessions(memory_session_id);
        CREATE INDEX IF NOT EXISTS idx_session_summaries_project_created ON session_summaries(project, created_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_entity_name ON entities(canonical_name COLLATE NOCASE);
        CREATE INDEX IF NOT EXISTS idx_memory_entities_entity ON memory_entities(entity_id);"
    )?;

    Ok(())
}

/// Try to add a column; silently ignore if it already exists.
fn add_column_if_missing(conn: &Connection, table: &str, column: &str, typedef: &str) {
    let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, typedef);
    if let Err(e) = conn.execute_batch(&sql) {
        let msg = e.to_string();
        if !msg.contains("duplicate column") {
            crate::log::warn(
                "migrate",
                &format!("backfill {}.{}: {}", table, column, msg),
            );
        }
    }
}

fn has_existing_migration_entries(conn: &Connection) -> bool {
    if !has_migration_table(conn) {
        return false;
    }

    conn.query_row("SELECT COUNT(*) FROM _schema_migrations", [], |row| {
        row.get(0)
    })
    .unwrap_or(0_i64)
        > 0
}
