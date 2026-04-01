use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

use super::types::{LoadedContext, SessionSummaryBrief};

pub(super) fn load_context_data(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
) -> LoadedContext {
    let mut memories = load_project_memories(conn, project);
    sort_memories_by_branch(&mut memories, current_branch);

    let summaries = query_recent_summaries(conn, project, 5).unwrap_or_default();
    let workstreams =
        crate::workstream::query_active_workstreams(conn, project).unwrap_or_default();

    LoadedContext {
        memories,
        summaries,
        workstreams,
    }
}

fn load_project_memories(conn: &Connection, project: &str) -> Vec<Memory> {
    let mut memories = Vec::new();
    let mut seen_ids = HashSet::new();

    let project_query = project.rsplit('/').next().unwrap_or(project);
    if let Ok(searched) =
        crate::search::search(conn, Some(project_query), Some(project), None, 20, 0, false)
    {
        for memory in searched {
            seen_ids.insert(memory.id);
            memories.push(memory);
        }
    }

    let recent = memory::get_recent_memories(conn, project, 50).unwrap_or_default();
    for memory in recent {
        if seen_ids.insert(memory.id) {
            memories.push(memory);
        }
    }

    memories.truncate(50);
    memories
}

fn sort_memories_by_branch(memories: &mut [Memory], current_branch: Option<&str>) {
    let Some(branch) = current_branch else {
        return;
    };

    memories.sort_by(|left, right| {
        branch_sort_score(left, branch).cmp(&branch_sort_score(right, branch))
    });
}

fn branch_sort_score(memory: &Memory, current_branch: &str) -> u8 {
    match memory.branch.as_deref() {
        Some(branch) if branch == current_branch => 0,
        None => 1,
        Some("main") | Some("master") => 2,
        _ => 3,
    }
}

pub(super) fn query_recent_summaries(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<Vec<SessionSummaryBrief>> {
    let mut stmt = conn.prepare(
        "SELECT request, completed, created_at_epoch \
         FROM session_summaries \
         WHERE project = ?1 AND request IS NOT NULL AND request != '' \
         ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![project, limit as i64], |row| {
        Ok(SessionSummaryBrief {
            request: row.get(0)?,
            completed: row.get(1)?,
            created_at_epoch: row.get(2)?,
        })
    })?;
    Ok(rows.flatten().collect())
}
