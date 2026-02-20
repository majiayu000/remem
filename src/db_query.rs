use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::db::Observation;
use crate::db::SessionSummary;

/// Shared row mapper — eliminates 5x duplication of Observation field extraction.
/// Expects columns: id, memory_session_id, type, title, subtitle, narrative,
/// facts, concepts, files_read, files_modified, discovery_tokens,
/// created_at, created_at_epoch, project, status, last_accessed_epoch
fn map_observation_row(row: &rusqlite::Row) -> rusqlite::Result<Observation> {
    Ok(Observation {
        id: row.get(0)?,
        memory_session_id: row.get(1)?,
        r#type: row.get(2)?,
        title: row.get(3)?,
        subtitle: row.get(4)?,
        narrative: row.get(5)?,
        facts: row.get(6)?,
        concepts: row.get(7)?,
        files_read: row.get(8)?,
        files_modified: row.get(9)?,
        discovery_tokens: row.get(10)?,
        created_at: row.get(11)?,
        created_at_epoch: row.get(12)?,
        project: row.get(13)?,
        status: row.get::<_, Option<String>>(14)?.unwrap_or_else(|| "active".to_string()),
        last_accessed_epoch: row.get(15)?,
    })
}

/// Same mapper but with project injected (for queries without project in SELECT).
fn map_observation_row_with_project(project: &str) -> impl Fn(&rusqlite::Row) -> rusqlite::Result<Observation> + '_ {
    move |row: &rusqlite::Row| {
        Ok(Observation {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            r#type: row.get(2)?,
            title: row.get(3)?,
            subtitle: row.get(4)?,
            narrative: row.get(5)?,
            facts: row.get(6)?,
            concepts: row.get(7)?,
            files_read: row.get(8)?,
            files_modified: row.get(9)?,
            discovery_tokens: row.get(10)?,
            created_at: row.get(11)?,
            created_at_epoch: row.get(12)?,
            project: Some(project.to_string()),
            status: row.get::<_, Option<String>>(13)?.unwrap_or_else(|| "active".to_string()),
            last_accessed_epoch: row.get(14)?,
        })
    }
}

const OBS_COLS: &str = "id, memory_session_id, type, title, subtitle, narrative, \
    facts, concepts, files_read, files_modified, discovery_tokens, \
    created_at, created_at_epoch, status, last_accessed_epoch";

const OBS_COLS_WITH_PROJECT: &str = "id, memory_session_id, type, title, subtitle, narrative, \
    facts, concepts, files_read, files_modified, discovery_tokens, \
    created_at, created_at_epoch, project, status, last_accessed_epoch";

fn collect_rows<T>(rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>>) -> Result<Vec<T>> {
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn open_db_readonly() -> Result<Connection> {
    let path = crate::db::db_path();
    let conn = Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("Failed to open database (readonly): {}", path.display()))?;
    Ok(conn)
}

pub fn query_observations(
    conn: &Connection,
    project: &str,
    types: &[&str],
    limit: i64,
) -> Result<Vec<Observation>> {
    if types.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = types.iter().enumerate().map(|(i, _)| format!("?{}", i + 2)).collect();
    let sql = format!(
        "SELECT {} FROM observations \
         WHERE project = ?1 AND type IN ({}) \
         ORDER BY created_at_epoch DESC LIMIT ?{}",
        OBS_COLS, placeholders.join(", "), types.len() + 2
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(project.to_string()));
    for t in types {
        param_values.push(Box::new(t.to_string()));
    }
    param_values.push(Box::new(limit));

    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), map_observation_row_with_project(project))?;
    collect_rows(rows)
}

pub fn query_summaries(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<SessionSummary>> {
    let mut stmt = conn.prepare(
        "SELECT id, memory_session_id, request, completed, decisions, learned, \
         next_steps, preferences, created_at, created_at_epoch \
         FROM session_summaries \
         WHERE project = ?1 \
         ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![project, limit], |row| {
        Ok(SessionSummary {
            id: row.get(0)?,
            memory_session_id: row.get(1)?,
            request: row.get(2)?,
            completed: row.get(3)?,
            decisions: row.get(4)?,
            learned: row.get(5)?,
            next_steps: row.get(6)?,
            preferences: row.get(7)?,
            created_at: row.get(8)?,
            created_at_epoch: row.get(9)?,
            project: Some(project.to_string()),
        })
    })?;
    collect_rows(rows)
}

pub fn search_observations_fts(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Observation>> {
    let mut conditions = vec!["observations_fts MATCH ?1".to_string()];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(query.to_string()));

    let mut idx = 2;
    if let Some(p) = project {
        conditions.push(format!("o.project = ?{idx}"));
        param_values.push(Box::new(p.to_string()));
        idx += 1;
    }
    if let Some(t) = obs_type {
        conditions.push(format!("o.type = ?{idx}"));
        param_values.push(Box::new(t.to_string()));
        idx += 1;
    }
    if !include_stale {
        conditions.push("o.status = 'active'".to_string());
    }

    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

    let sql = format!(
        "SELECT o.id, o.memory_session_id, o.type, o.title, o.subtitle, o.narrative, \
         o.facts, o.concepts, o.files_read, o.files_modified, o.discovery_tokens, \
         o.created_at, o.created_at_epoch, o.project, o.status, o.last_accessed_epoch \
         FROM observations o \
         JOIN observations_fts ON observations_fts.rowid = o.id \
         WHERE {} \
         ORDER BY (\
           rank * (1.0 + 0.5 * (strftime('%s','now') - o.created_at_epoch) / 2592000.0) \
           + CASE WHEN o.status = 'stale' THEN 1000.0 ELSE 0.0 END\
         ) \
         LIMIT ?{} OFFSET ?{}",
        conditions.join(" AND "), idx, idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

pub fn get_observations_by_ids(
    conn: &Connection,
    ids: &[i64],
) -> Result<Vec<Observation>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "SELECT {} FROM observations WHERE id IN ({}) \
         ORDER BY created_at_epoch DESC",
        OBS_COLS_WITH_PROJECT, placeholders.join(", ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        ids.iter().map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>).collect();
    let refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

/// Count active observations for a project.
pub fn count_active_observations(conn: &Connection, project: &str) -> Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM observations WHERE project = ?1 AND status IN ('active', 'stale')",
        params![project],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Get oldest observations for compression.
pub fn get_oldest_observations(
    conn: &Connection,
    project: &str,
    keep: i64,
    batch_size: i64,
) -> Result<Vec<Observation>> {
    let total = count_active_observations(conn, project)?;
    let compressible = total - keep;
    if compressible <= 0 {
        return Ok(vec![]);
    }
    let take = compressible.min(batch_size);

    let sql = format!(
        "SELECT {} FROM observations \
         WHERE project = ?1 AND status IN ('active', 'stale') \
         ORDER BY created_at_epoch ASC LIMIT ?2",
        OBS_COLS
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![project, take], map_observation_row_with_project(project))?;
    collect_rows(rows)
}

pub fn get_timeline_around(
    conn: &Connection,
    anchor_id: i64,
    depth_before: i64,
    depth_after: i64,
    project: Option<&str>,
) -> Result<Vec<Observation>> {
    let anchor_sql = format!(
        "SELECT {} FROM observations WHERE id = ?1",
        OBS_COLS_WITH_PROJECT
    );
    let anchor: Observation = conn.query_row(&anchor_sql, params![anchor_id], map_observation_row)?;
    let epoch = anchor.created_at_epoch;

    let project_filter = if project.is_some() { " AND project = ?3" } else { "" };

    let before_sql = format!(
        "SELECT {} FROM observations \
         WHERE created_at_epoch < ?1{} \
         ORDER BY created_at_epoch DESC LIMIT ?2",
        OBS_COLS_WITH_PROJECT, project_filter
    );
    let after_sql = format!(
        "SELECT {} FROM observations \
         WHERE created_at_epoch > ?1{} \
         ORDER BY created_at_epoch ASC LIMIT ?2",
        OBS_COLS_WITH_PROJECT, project_filter
    );

    let mut result = Vec::new();

    for (sql, depth) in [(&before_sql, depth_before), (&after_sql, depth_after)] {
        let mut stmt = conn.prepare(sql)?;
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(epoch),
            Box::new(depth),
        ];
        if let Some(p) = project {
            params_vec.push(Box::new(p.to_string()));
        }
        let refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
        for row in rows {
            result.push(row?);
        }
    }

    result.push(anchor);
    result.sort_by_key(|o| o.created_at_epoch);
    Ok(result)
}
