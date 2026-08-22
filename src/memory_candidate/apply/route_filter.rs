use anyhow::Result;
use rusqlite::{params, Connection};

use super::CandidateRoute;

pub(super) fn matches_active_route(
    conn: &Connection,
    memory_id: i64,
    project: &str,
    scope: &str,
    route: &CandidateRoute,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM memories
             WHERE id = ?1 AND status = 'active' AND project = ?2
               AND branch IS NULL AND COALESCE(scope, 'project') = ?3
               AND COALESCE(owner_scope,
                   CASE WHEN COALESCE(scope, 'project') = 'global'
                        THEN 'user' ELSE 'repo' END) = ?4
               AND COALESCE(owner_key,
                   CASE WHEN COALESCE(scope, 'project') = 'global'
                        THEN 'user:default' ELSE project END) = ?5
               AND target_project IS ?6
         )",
        params![
            memory_id,
            project,
            scope,
            route.owner_scope,
            route.owner_key,
            route.target_project,
        ],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub(super) fn ids(
    conn: &Connection,
    candidates: Vec<i64>,
    project: &str,
    scope: &str,
    route: &CandidateRoute,
) -> Result<Vec<i64>> {
    let mut filtered = Vec::with_capacity(candidates.len());
    for memory_id in candidates {
        if matches_active_route(conn, memory_id, project, scope, route)? {
            filtered.push(memory_id);
        }
    }
    Ok(filtered)
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn filters_same_owner_rows_outside_project_and_branch_route() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        for (project, branch) in [("/repo", None), ("/repo", Some("main")), ("/other", None)] {
            conn.execute(
                "INSERT INTO memories
                 (project, title, content, memory_type, created_at_epoch,
                  updated_at_epoch, status, branch, scope, source_project,
                  target_project, owner_scope, owner_key, source_trust_class)
                 VALUES (?1, 'title', 'content', 'discovery', 1, 1, 'active',
                         ?2, 'project', ?1, '/repo', 'repo', '/repo', 'external_content')",
                params![project, branch],
            )?;
        }
        let route = CandidateRoute {
            owner_scope: "repo".to_string(),
            owner_key: "/repo".to_string(),
            target_project: Some("/repo".to_string()),
            topic_domain: None,
            routing_confidence: 1.0,
            routing_reason: "test".to_string(),
            context_class: "startup_core".to_string(),
        };
        assert_eq!(
            ids(&conn, vec![1, 2, 3], "/repo", "project", &route)?,
            vec![1]
        );
        Ok(())
    }
}
