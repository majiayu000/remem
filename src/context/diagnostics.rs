use anyhow::Result;
use rusqlite::Connection;

use crate::memory;

use super::types::{
    ContextDiagnostics, ContextExclusion, HiddenDuplicateGroup, StateKeyDiagnosticGroup,
};

pub(super) fn collect_context_diagnostics(
    conn: &Connection,
    project: &str,
    excluded_types: &[&str],
    selected_ids: Vec<i64>,
    hidden_duplicate_groups: Vec<HiddenDuplicateGroup>,
) -> ContextDiagnostics {
    let candidate_pool_total = count_context_candidate_pool(conn, project, excluded_types)
        .unwrap_or_else(|error| {
            crate::log::error(
                "context",
                &format!("failed to count context candidate pool for {project}: {error}"),
            );
            0
        });
    let exclusions = query_context_lifecycle_exclusions(conn, project, excluded_types, 60)
        .unwrap_or_else(|error| {
            crate::log::error(
                "context",
                &format!("failed to load context lifecycle diagnostics for {project}: {error}"),
            );
            Vec::new()
        });
    let state_key_groups = query_state_key_diagnostics(conn, project, excluded_types, 60)
        .unwrap_or_else(|error| {
            crate::log::error(
                "context",
                &format!("failed to load context state-key diagnostics for {project}: {error}"),
            );
            Vec::new()
        });

    ContextDiagnostics {
        candidate_pool_total,
        current_rows: selected_ids.len(),
        selected_ids,
        hidden_duplicate_groups,
        preference_selected_ids: Vec::new(),
        preference_hidden_duplicate_groups: Vec::new(),
        preference_state_key_groups: Vec::new(),
        state_key_groups,
        exclusions,
    }
}

pub(super) fn apply_preference_diagnostics(
    conn: &Connection,
    project: &str,
    rendered_ids: Vec<i64>,
    diagnostics: &mut ContextDiagnostics,
) {
    diagnostics.preference_selected_ids = rendered_ids;
    diagnostics.preference_hidden_duplicate_groups =
        query_preference_topic_duplicate_groups(conn, project, 60).unwrap_or_else(|error| {
            crate::log::error(
                "context",
                &format!("failed to load preference duplicate diagnostics for {project}: {error}"),
            );
            Vec::new()
        });
    diagnostics.preference_state_key_groups =
        query_preference_state_key_diagnostics(conn, project, 60).unwrap_or_else(|error| {
            crate::log::error(
                "context",
                &format!("failed to load preference state-key diagnostics for {project}: {error}"),
            );
            Vec::new()
        });
}

fn count_context_candidate_pool(
    conn: &Connection,
    project: &str,
    excluded_types: &[&str],
) -> Result<usize> {
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    super::query::push_owner_included_filter(project, &mut idx, &mut conditions, &mut params);
    super::query::push_excluded_type_filter(excluded_types, &mut idx, &mut conditions, &mut params);

    let sql = format!(
        "SELECT COUNT(*) FROM memories WHERE {}",
        conditions.join(" AND ")
    );
    let refs = crate::db::to_sql_refs(&params);
    let count = conn.query_row(&sql, refs.as_slice(), |row| row.get::<_, i64>(0))?;
    Ok(count.max(0) as usize)
}

fn query_context_lifecycle_exclusions(
    conn: &Connection,
    project: &str,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<ContextExclusion>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }

    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;
    super::query::push_owner_included_filter(project, &mut idx, &mut conditions, &mut params);
    super::query::push_excluded_type_filter(excluded_types, &mut idx, &mut conditions, &mut params);
    conditions.push(
        "(status IN ('stale', 'superseded') \
          OR (status = 'active' AND expires_at_epoch IS NOT NULL \
              AND expires_at_epoch <= CAST(strftime('%s', 'now') AS INTEGER)))"
            .to_string(),
    );
    params.push(Box::new(limit));

    let sql = format!(
        "SELECT id, title, status, expires_at_epoch FROM memories \
         WHERE {} ORDER BY updated_at_epoch DESC LIMIT ?{}",
        conditions.join(" AND "),
        idx,
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        let status: String = row.get(2)?;
        let expires_at_epoch: Option<i64> = row.get(3)?;
        let reason = if status == "superseded" {
            "superseded"
        } else if status != "active" {
            "stale"
        } else if expires_at_epoch.is_some() {
            "expired"
        } else {
            "stale"
        };
        Ok(ContextExclusion {
            id: row.get(0)?,
            title: row.get(1)?,
            reason,
            status,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

fn query_preference_topic_duplicate_groups(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<Vec<HiddenDuplicateGroup>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT
            COALESCE(m.owner_scope, CASE WHEN m.scope = 'global' THEN 'legacy_global' ELSE 'legacy_project' END)
            || ':' ||
            COALESCE(m.owner_key, CASE WHEN m.scope = 'global' THEN 'global' ELSE m.project END)
            || ':topic:' || m.topic_key AS cluster_key,
            m.id
         FROM memories m
         WHERE m.memory_type = 'preference'
           AND {}
           AND {}
           AND m.topic_key IS NOT NULL
           AND {}
         ORDER BY cluster_key ASC, m.updated_at_epoch DESC, m.id DESC
         LIMIT ?2",
        memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false),
        memory::memory_state_key_current_filter_sql("m"),
        preference_context_owner_filter_sql("m"),
    );
    let row_limit = (limit * 8).max(limit);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![project, row_limit as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let rows = crate::db::query::collect_rows(rows)?;
    Ok(build_duplicate_groups(rows, limit))
}

fn query_preference_state_key_diagnostics(
    conn: &Connection,
    project: &str,
    limit: i64,
) -> Result<Vec<StateKeyDiagnosticGroup>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }

    let sql = format!(
        "SELECT sk.owner_scope, sk.owner_key, sk.memory_type, sk.state_key,
                sk.current_memory_id, GROUP_CONCAT(m.id), COUNT(*),
                MAX(m.updated_at_epoch)
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.memory_type = 'preference'
           AND {}
           AND {}
         GROUP BY sk.id
         HAVING COUNT(*) > 1
         ORDER BY MAX(m.updated_at_epoch) DESC
         LIMIT ?2",
        memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false),
        preference_context_owner_filter_sql("m"),
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![project, limit], |row| {
        let id_list: String = row.get(5)?;
        let mut active_ids = parse_id_list(&id_list).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    error.to_string(),
                )),
            )
        })?;
        active_ids.sort_unstable();
        Ok(StateKeyDiagnosticGroup {
            owner_scope: row.get(0)?,
            owner_key: row.get(1)?,
            memory_type: row.get(2)?,
            state_key: row.get(3)?,
            current_id: row.get(4)?,
            active_ids,
            reason: "ambiguous_active_state_key_group",
        })
    })?;
    crate::db::query::collect_rows(rows)
}

fn query_state_key_diagnostics(
    conn: &Connection,
    project: &str,
    excluded_types: &[&str],
    limit: i64,
) -> Result<Vec<StateKeyDiagnosticGroup>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(project.to_string()),
        Box::new(project.to_string()),
        Box::new(project.to_string()),
    ];
    let mut excluded_filter = String::new();
    for memory_type in excluded_types {
        let idx = params.len() + 1;
        excluded_filter.push_str(&format!(" AND m.memory_type <> ?{idx}"));
        params.push(Box::new((*memory_type).to_string()));
    }
    let limit_idx = params.len() + 1;
    params.push(Box::new(limit));

    let sql = format!(
        "SELECT sk.owner_scope, sk.owner_key, sk.memory_type, sk.state_key,
                sk.current_memory_id, GROUP_CONCAT(m.id), COUNT(*),
                MAX(m.updated_at_epoch)
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE ((m.owner_scope = 'repo' AND m.owner_key = ?1)
                OR (m.owner_scope = 'repo' AND m.target_project = ?2)
                OR (m.owner_scope IS NULL AND m.project = ?3
                    AND COALESCE(m.scope, 'project') != 'global'))
           AND m.status = 'active'
           AND (m.expires_at_epoch IS NULL
                OR m.expires_at_epoch > CAST(strftime('%s', 'now') AS INTEGER))
           {excluded_filter}
         GROUP BY sk.id
         HAVING COUNT(*) > 1
         ORDER BY MAX(m.updated_at_epoch) DESC
         LIMIT ?{limit_idx}"
    );
    let refs = crate::db::to_sql_refs(&params);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        let id_list: String = row.get(5)?;
        let mut active_ids = parse_id_list(&id_list).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    error.to_string(),
                )),
            )
        })?;
        active_ids.sort_unstable();
        Ok(StateKeyDiagnosticGroup {
            owner_scope: row.get(0)?,
            owner_key: row.get(1)?,
            memory_type: row.get(2)?,
            state_key: row.get(3)?,
            current_id: row.get(4)?,
            active_ids,
            reason: "ambiguous_active_state_key_group",
        })
    })?;
    crate::db::query::collect_rows(rows)
}

fn build_duplicate_groups(rows: Vec<(String, i64)>, limit: usize) -> Vec<HiddenDuplicateGroup> {
    let mut groups = Vec::new();
    let mut current_key: Option<String> = None;
    let mut current_ids = Vec::new();
    for (cluster_key, id) in rows {
        if current_key.as_deref() != Some(cluster_key.as_str()) {
            push_duplicate_group(&mut groups, current_key.take(), &mut current_ids);
            current_key = Some(cluster_key);
            if groups.len() >= limit {
                return groups;
            }
        }
        current_ids.push(id);
    }
    push_duplicate_group(&mut groups, current_key, &mut current_ids);
    groups.truncate(limit);
    groups
}

fn push_duplicate_group(
    groups: &mut Vec<HiddenDuplicateGroup>,
    cluster_key: Option<String>,
    ids: &mut Vec<i64>,
) {
    if let Some(cluster_key) = cluster_key {
        if ids.len() > 1 {
            groups.push(HiddenDuplicateGroup {
                cluster_key,
                chosen_id: ids[0],
                hidden_ids: ids[1..].to_vec(),
            });
        }
    }
    ids.clear();
}

fn preference_context_owner_filter_sql(alias: &str) -> String {
    format!(
        "(({alias}.owner_scope = 'repo' AND ({alias}.owner_key = ?1 OR {alias}.target_project = ?1))
          OR ({alias}.owner_scope IS NULL AND {alias}.project = ?1
              AND ({alias}.scope IS NULL OR {alias}.scope = 'project'))
          OR ({alias}.owner_scope = 'user' AND {alias}.owner_key = 'user:default')
          OR ({alias}.owner_scope IS NULL AND {alias}.scope = 'global'))"
    )
}

fn parse_id_list(value: &str) -> Result<Vec<i64>> {
    value
        .split(',')
        .map(|part| {
            part.trim()
                .parse::<i64>()
                .map_err(|error| anyhow::anyhow!("invalid state-key id {part}: {error}"))
        })
        .collect()
}
