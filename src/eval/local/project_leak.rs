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
                let (memory_project, scope): (String, String) = conn.query_row(
                    "SELECT project, scope FROM memories WHERE id = ?1",
                    params![memory_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                total_hits += 1;
                if scope == "global" {
                    global_overlay_hits += 1;
                } else if !crate::project_id::project_matches(Some(&memory_project), project) {
                    leaked += 1;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE memories (
                id INTEGER PRIMARY KEY,
                project TEXT NOT NULL,
                memory_type TEXT NOT NULL DEFAULT 'discovery',
                status TEXT NOT NULL DEFAULT 'active',
                scope TEXT NOT NULL DEFAULT 'project',
                branch TEXT,
                expires_at_epoch INTEGER,
                updated_at_epoch INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE entities (
                id INTEGER PRIMARY KEY,
                canonical_name TEXT NOT NULL UNIQUE COLLATE NOCASE
            );
            CREATE TABLE memory_entities (
                memory_id INTEGER NOT NULL,
                entity_id INTEGER NOT NULL,
                UNIQUE(memory_id, entity_id)
            );",
        )?;
        Ok(())
    }

    fn link_entity(conn: &Connection, memory_id: i64, entity_id: i64, name: &str) -> Result<()> {
        conn.execute(
            "INSERT OR IGNORE INTO entities (id, canonical_name) VALUES (?1, ?2)",
            params![entity_id, name],
        )?;
        conn.execute(
            "INSERT INTO memory_entities (memory_id, entity_id) VALUES (?1, ?2)",
            params![memory_id, entity_id],
        )?;
        Ok(())
    }

    #[test]
    fn project_leak_counts_same_project_global_memory_as_overlay() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_schema(&conn)?;
        conn.execute(
            "INSERT INTO memories (id, project, scope, status) VALUES (?1, ?2, ?3, 'active')",
            params![1_i64, "proj-a", "project"],
        )?;
        conn.execute(
            "INSERT INTO memories (id, project, scope, status) VALUES (?1, ?2, ?3, 'active')",
            params![2_i64, "proj-b", "project"],
        )?;
        conn.execute(
            "INSERT INTO memories (id, project, scope, status) VALUES (?1, ?2, ?3, 'active')",
            params![3_i64, "proj-a", "global"],
        )?;
        link_entity(&conn, 1, 1, "SharedEntity")?;
        link_entity(&conn, 2, 2, "OtherEntity")?;
        link_entity(&conn, 3, 1, "SharedEntity")?;

        let report = check_project_leak(&conn)?;

        assert_eq!(report.total_hits, 3);
        assert_eq!(report.project_hits, 2);
        assert_eq!(report.global_overlay_hits, 1);
        assert_eq!(report.leaked, 0);
        Ok(())
    }
}
