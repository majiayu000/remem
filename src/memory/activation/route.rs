use anyhow::{Context, Result};
use rusqlite::Connection;

use super::ActiveMemoryRoute;
use crate::memory::poisoning::SourceTrustClass;

pub(crate) struct ExistingActivationRoute {
    pub(crate) route: ActiveMemoryRoute,
    pub(crate) source_project: String,
    pub(crate) result_source_trust: SourceTrustClass,
}

pub(crate) fn load_existing_route(
    conn: &Connection,
    memory_id: i64,
) -> Result<ExistingActivationRoute> {
    let (route, source_project, trust): (ActiveMemoryRoute, String, String) = conn.query_row(
        "SELECT project, branch, COALESCE(scope, 'project'),
                COALESCE(owner_scope,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END),
                COALESCE(owner_key,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END),
                CASE
                    WHEN COALESCE(owner_scope,
                        CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END) = 'repo'
                    THEN COALESCE(target_project, project)
                    ELSE target_project
                END,
                COALESCE(source_project, project), source_trust_class
         FROM memories WHERE id = ?1",
        [memory_id],
        |row| {
            Ok((
                ActiveMemoryRoute {
                    project: row.get(0)?,
                    branch: row.get(1)?,
                    scope: row.get(2)?,
                    owner_scope: row.get(3)?,
                    owner_key: row.get(4)?,
                    target_project: row.get(5)?,
                },
                row.get(6)?,
                row.get(7)?,
            ))
        },
    )?;
    let result_source_trust = SourceTrustClass::parse(&trust)
        .context("existing active-memory route has invalid result trust")?;
    Ok(ExistingActivationRoute {
        route,
        source_project,
        result_source_trust,
    })
}
