use anyhow::Result;
use rusqlite::{params, Connection};

use super::types::ProjectLeakReport;

pub(super) fn check_project_leak(conn: &Connection) -> Result<ProjectLeakReport> {
    let projects = top_projects(conn)?;
    if projects.len() < 2 {
        return Ok(ProjectLeakReport {
            total_tested: 0,
            total_hits: 0,
            project_hits: 0,
            global_overlay_hits: 0,
            leaked: 0,
            leak_rate: 0.0,
        });
    }

    let mut total_tested = 0;
    let mut total_hits = 0;
    let mut project_hits = 0;
    let mut global_overlay_hits = 0;
    let mut leaked = 0;

    for project in &projects {
        for entity in project_entities(conn, project)? {
            for memory_id in
                crate::retrieval::entity::search_by_entity(conn, &entity, Some(project), 20)?
            {
                let (memory_project, scope): (String, String) = conn
                    .query_row(
                        "SELECT project, scope FROM memories WHERE id = ?1",
                        params![memory_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap_or_default();
                total_hits += 1;
                if !crate::project_id::project_matches(Some(&memory_project), project) {
                    if scope == "global" {
                        global_overlay_hits += 1;
                    } else {
                        leaked += 1;
                    }
                } else {
                    project_hits += 1;
                }
            }
            total_tested += 1;
        }
    }

    let leak_rate = if total_hits > 0 {
        leaked as f64 / total_hits as f64
    } else {
        0.0
    };
    Ok(ProjectLeakReport {
        total_tested,
        total_hits,
        project_hits,
        global_overlay_hits,
        leaked,
        leak_rate,
    })
}

fn top_projects(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT project, COUNT(*) as cnt FROM memories
         WHERE status = 'active' AND project != ''
         GROUP BY project ORDER BY cnt DESC LIMIT 5",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.flatten().collect())
}

fn project_entities(conn: &Connection, project: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT e.canonical_name FROM memory_entities me
         JOIN entities e ON e.id = me.entity_id
         JOIN memories m ON m.id = me.memory_id
         WHERE m.project = ?1 AND m.status = 'active'
         LIMIT 3",
    )?;
    let rows = stmt.query_map(params![project], |row| row.get::<_, String>(0))?;
    Ok(rows.flatten().collect())
}
