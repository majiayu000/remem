use anyhow::Result;
use rusqlite::Connection;

use super::dedup::check_dedup;
use super::project_leak::check_project_leak;
use super::self_retrieval::check_self_retrieval;
use super::title_quality::check_title_quality;
use super::types::EvalReport;

pub fn run_eval(conn: &Connection) -> Result<EvalReport> {
    let total_memories: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;

    Ok(EvalReport {
        total_memories,
        dedup: check_dedup(conn)?,
        project_leak: check_project_leak(conn)?,
        title_quality: check_title_quality(conn)?,
        self_retrieval: check_self_retrieval(conn)?,
    })
}
