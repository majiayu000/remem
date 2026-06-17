use anyhow::Result;
use rusqlite::Connection;

#[derive(Debug)]
pub(super) struct StalenessCapabilities {
    pub(super) git_trace_tables_exist: bool,
    pub(super) memories_exists: bool,
    pub(super) memories_source_project: bool,
    pub(super) memories_evidence_event_ids: bool,
    pub(super) memories_source_candidate_id: bool,
    pub(super) memory_candidates_exists: bool,
    pub(super) memory_candidates_evidence_event_ids: bool,
    pub(super) projects_exists: bool,
    pub(super) captured_events_exists: bool,
    pub(super) captured_events_reference_time_epoch: bool,
    pub(super) events_exists: bool,
    pub(super) observations_exists: bool,
    pub(super) observations_session_row_id: bool,
    pub(super) observations_evidence_event_ids: bool,
}

impl StalenessCapabilities {
    pub(super) fn load(conn: &Connection) -> Result<Self> {
        let memories_exists = table_exists(conn, "memories")?;
        let memory_candidates_exists = table_exists(conn, "memory_candidates")?;
        let projects_exists = table_exists(conn, "projects")?;
        let captured_events_exists = table_exists(conn, "captured_events")?;
        let observations_exists = table_exists(conn, "observations")?;
        Ok(Self {
            git_trace_tables_exist: git_trace_tables_exist(conn)?,
            memories_exists,
            memories_source_project: memories_exists
                && column_exists(conn, "memories", "source_project")?,
            memories_evidence_event_ids: memories_exists
                && column_exists(conn, "memories", "evidence_event_ids")?,
            memories_source_candidate_id: memories_exists
                && column_exists(conn, "memories", "source_candidate_id")?,
            memory_candidates_exists,
            memory_candidates_evidence_event_ids: memory_candidates_exists
                && column_exists(conn, "memory_candidates", "evidence_event_ids")?,
            projects_exists,
            captured_events_exists,
            captured_events_reference_time_epoch: captured_events_exists
                && column_exists(conn, "captured_events", "reference_time_epoch")?,
            events_exists: table_exists(conn, "events")?,
            observations_exists,
            observations_session_row_id: observations_exists
                && column_exists(conn, "observations", "session_row_id")?,
            observations_evidence_event_ids: observations_exists
                && column_exists(conn, "observations", "evidence_event_ids")?,
        })
    }
}

fn git_trace_tables_exist(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM sqlite_master
         WHERE type = 'table'
           AND name IN ('git_commits', 'git_commit_sessions')",
        [],
        |row| row.get(0),
    )?;
    Ok(count == 2)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let exists = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(exists != 0)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}
