use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::Observation;
use crate::db::SessionSummary;

/// Shared row mapper — eliminates 5x duplication of Observation field extraction.
/// Expects columns: id, memory_session_id, type, title, subtitle, narrative,
/// facts, concepts, files_read, files_modified, discovery_tokens,
/// created_at, created_at_epoch, project, status, last_accessed_epoch,
/// content_session_id
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
        status: row
            .get::<_, Option<String>>(14)?
            .unwrap_or_else(|| "active".to_string()),
        last_accessed_epoch: row.get(15)?,
        content_session_id: row.get(16)?,
        branch: row.get(17)?,
        commit_sha: row.get(18)?,
    })
}

/// 旧版 claude-mem 用毫秒 epoch，remem 用秒 epoch。
/// 秒级 epoch 当前 ~1.7×10⁹，毫秒级 ~1.7×10¹²。以 10¹⁰ 为分界线排除旧数据。
const EPOCH_SECS_ONLY: &str = "created_at_epoch < 10000000000";

/// Build SELECT column list for observations, including content_session_id via subquery.
/// `table_ref` is the table name or alias (e.g. "observations" or "o").
fn obs_select_cols(table_ref: &str) -> String {
    format!(
        "{t}.id, {t}.memory_session_id, {t}.type, {t}.title, {t}.subtitle, {t}.narrative, \
         {t}.facts, {t}.concepts, {t}.files_read, {t}.files_modified, {t}.discovery_tokens, \
         {t}.created_at, {t}.created_at_epoch, {t}.project, {t}.status, {t}.last_accessed_epoch, \
         (SELECT s.content_session_id FROM sdk_sessions s \
          WHERE s.memory_session_id = {t}.memory_session_id LIMIT 1) AS content_session_id, \
         {t}.branch, {t}.commit_sha",
        t = table_ref
    )
}

pub fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn push_project_filter(
    column: &str,
    project: &str,
    idx: usize,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> (String, usize) {
    crate::project_id::push_project_filter(column, project, idx, params)
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

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, mut idx) = push_project_filter("project", project, 1, &mut param_values);

    let placeholders: Vec<String> = types
        .iter()
        .map(|_| {
            let p = format!("?{idx}");
            idx += 1;
            p
        })
        .collect();
    for t in types {
        param_values.push(Box::new(t.to_string()));
    }
    param_values.push(Box::new(limit));

    let sql = format!(
        "SELECT {} FROM observations \
         WHERE {} AND {} AND type IN ({}) \
         ORDER BY created_at_epoch DESC LIMIT ?{}",
        obs_select_cols("observations"),
        project_filter,
        EPOCH_SECS_ONLY,
        placeholders.join(", "),
        idx
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

pub fn query_summaries(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<SessionSummary>> {
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, idx) = push_project_filter("project", project, 1, &mut param_values);
    param_values.push(Box::new(limit));

    let mut stmt = conn.prepare(&format!(
        "SELECT id, memory_session_id, request, completed, decisions, learned, \
         next_steps, preferences, created_at, created_at_epoch, project \
         FROM session_summaries \
         WHERE {} AND {} \
         ORDER BY created_at_epoch DESC LIMIT ?{}",
        project_filter, EPOCH_SECS_ONLY, idx
    ))?;

    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), |row| {
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
            project: row.get(10)?,
        })
    })?;
    collect_rows(rows)
}

/// Get the latest summary for a given memory_session_id + project.
pub fn get_summary_by_session(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
) -> Result<Option<SessionSummary>> {
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(memory_session_id.to_string()));
    let (project_filter, _next_idx) = push_project_filter("project", project, 2, &mut param_values);

    let mut stmt = conn.prepare(&format!(
        "SELECT id, memory_session_id, request, completed, decisions, learned, \
             next_steps, preferences, created_at, created_at_epoch, project \
             FROM session_summaries \
             WHERE memory_session_id = ?1 AND {} AND {} \
             ORDER BY created_at_epoch DESC LIMIT 1",
        project_filter, EPOCH_SECS_ONLY
    ))?;

    let refs = crate::db::to_sql_refs(&param_values);
    let mut rows = stmt.query_map(refs.as_slice(), |row| {
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
            project: row.get(10)?,
        })
    })?;

    match rows.next() {
        Some(Ok(s)) => Ok(Some(s)),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
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
    let mut conditions = vec![
        "observations_fts MATCH ?1".to_string(),
        format!("o.{}", EPOCH_SECS_ONLY),
    ];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(query.to_string()));

    let mut idx = 2;
    if let Some(p) = project {
        let (project_filter, next_idx) =
            push_project_filter("o.project", p, idx, &mut param_values);
        conditions.push(project_filter);
        idx = next_idx;
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
        "SELECT {} \
         FROM observations o \
         JOIN observations_fts ON observations_fts.rowid = o.id \
         WHERE {} \
         ORDER BY (\
           ((-rank) / (\
             1.0 + 0.5 * (CASE \
               WHEN (strftime('%s','now') - o.created_at_epoch) > 0 \
                 THEN (strftime('%s','now') - o.created_at_epoch) \
               ELSE 0 \
             END) / 2592000.0\
           )) * CASE WHEN o.status = 'stale' THEN 0.25 ELSE 1.0 END\
         ) DESC \
         LIMIT ?{} OFFSET ?{}",
        obs_select_cols("o"),
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

/// LIKE-based fallback search for queries with tokens shorter than 3 characters.
/// Each token must appear in at least one text column (title/subtitle/narrative/facts/concepts).
pub fn search_observations_like(
    conn: &Connection,
    tokens: &[&str],
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Observation>> {
    if tokens.is_empty() {
        return Ok(vec![]);
    }

    let mut conditions = vec![format!("o.{}", EPOCH_SECS_ONLY)];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    for token in tokens {
        let like_pattern = format!("%{token}%");
        let cols = [
            "o.title",
            "o.subtitle",
            "o.narrative",
            "o.facts",
            "o.concepts",
        ];
        let token_clauses: Vec<String> = cols
            .iter()
            .map(|col| format!("{col} LIKE ?{idx}"))
            .collect();
        param_values.push(Box::new(like_pattern));
        conditions.push(format!("({})", token_clauses.join(" OR ")));
        idx += 1;
    }

    if let Some(p) = project {
        let (project_filter, next_idx) =
            push_project_filter("o.project", p, idx, &mut param_values);
        conditions.push(project_filter);
        idx = next_idx;
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
        "SELECT {} FROM observations o \
         WHERE {} \
         ORDER BY o.created_at_epoch DESC \
         LIMIT ?{} OFFSET ?{}",
        obs_select_cols("o"),
        conditions.join(" AND "),
        idx,
        idx + 1
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

pub fn get_observations_by_ids(
    conn: &Connection,
    ids: &[i64],
    project: Option<&str>,
) -> Result<Vec<Observation>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let mut conditions = vec![
        format!("id IN ({})", placeholders.join(", ")),
        EPOCH_SECS_ONLY.to_string(),
    ];
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    if let Some(p) = project {
        let (project_filter, _idx) =
            push_project_filter("project", p, ids.len() + 1, &mut param_values);
        conditions.push(project_filter);
    }
    let sql = format!(
        "SELECT {} FROM observations WHERE {} \
         ORDER BY created_at_epoch DESC",
        obs_select_cols("observations"),
        conditions.join(" AND ")
    );

    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
    collect_rows(rows)
}

/// Count active observations for a project.
pub fn count_active_observations(conn: &Connection, project: &str) -> Result<i64> {
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, _next_idx) = push_project_filter("project", project, 1, &mut param_values);
    let sql = format!(
        "SELECT COUNT(*) FROM observations \
         WHERE {} AND {} AND status IN ('active', 'stale')",
        project_filter, EPOCH_SECS_ONLY
    );
    let refs = crate::db::to_sql_refs(&param_values);
    let count: i64 = conn.query_row(&sql, refs.as_slice(), |row| row.get(0))?;
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

    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (project_filter, idx) = push_project_filter("project", project, 1, &mut param_values);
    param_values.push(Box::new(take));

    let sql = format!(
        "SELECT {} FROM observations \
         WHERE {} AND {} AND status IN ('active', 'stale') \
         ORDER BY created_at_epoch ASC LIMIT ?{}",
        obs_select_cols("observations"),
        project_filter,
        EPOCH_SECS_ONLY,
        idx
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
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
        obs_select_cols("observations")
    );
    let anchor: Observation =
        conn.query_row(&anchor_sql, params![anchor_id], map_observation_row)?;
    let epoch = anchor.created_at_epoch;

    let build_sql = |is_before: bool, project_filter: Option<&str>| -> String {
        let cmp = if is_before { "<" } else { ">" };
        let order = if is_before { "DESC" } else { "ASC" };
        let extra = project_filter
            .map(|f| format!(" AND {f}"))
            .unwrap_or_default();
        format!(
            "SELECT {} FROM observations \
             WHERE {} AND created_at_epoch {} ?1{} \
             ORDER BY created_at_epoch {} LIMIT ?2",
            obs_select_cols("observations"),
            EPOCH_SECS_ONLY,
            cmp,
            extra,
            order
        )
    };

    let mut result = Vec::new();

    for (is_before, depth) in [(true, depth_before), (false, depth_after)] {
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(epoch), Box::new(depth)];
        let project_filter = if let Some(p) = project {
            let (f, _next_idx) = push_project_filter("project", p, 3, &mut params_vec);
            Some(f)
        } else {
            None
        };
        let sql = build_sql(is_before, project_filter.as_deref());
        let mut stmt = conn.prepare(&sql)?;
        let refs = crate::db::to_sql_refs(&params_vec);
        let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
        for row in rows {
            result.push(row?);
        }
    }

    result.push(anchor);
    result.sort_by_key(|o| o.created_at_epoch);
    Ok(result)
}
