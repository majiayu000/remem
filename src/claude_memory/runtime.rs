use anyhow::Result;
use rusqlite::params;

use crate::claude_memory::index::ensure_memory_index;
use crate::claude_memory::paths::claude_memory_dir;
use crate::claude_memory::render::{max_sessions, render_memory_content, SessionRow, REMEM_FILE};

pub fn sync_to_claude_memory(cwd: &str, project: &str) -> Result<()> {
    let conn = crate::db::open_db()?;
    let memory_dir = claude_memory_dir(cwd);

    if !memory_dir.exists() {
        crate::log::info(
            "claude-mem",
            &format!("skip: no claude memory dir for {}", project),
        );
        return Ok(());
    }

    let sessions = load_recent_sessions(&conn, project)?;
    let Some(content) = render_memory_content(&sessions) else {
        return Ok(());
    };

    let file_path = memory_dir.join(REMEM_FILE);
    std::fs::write(&file_path, &content)?;
    ensure_memory_index(&memory_dir)?;

    let decisions_count = content.matches("\n- ").count();
    crate::log::info(
        "claude-mem",
        &format!(
            "synced {} sessions + {} decisions to {}",
            sessions.len(),
            decisions_count,
            file_path.display()
        ),
    );

    Ok(())
}

fn load_recent_sessions(conn: &rusqlite::Connection, project: &str) -> Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT request, completed, decisions, created_at_epoch \
         FROM session_summaries \
         WHERE project = ?1 AND request IS NOT NULL AND request != '' \
         ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![project, max_sessions() as i64], |row| {
        Ok(SessionRow {
            request: row.get(0)?,
            completed: row.get(1)?,
            decisions: row.get(2)?,
            created_at_epoch: row.get(3)?,
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }
    Ok(sessions)
}
