use anyhow::Result;
use rusqlite::Connection;

use crate::memory::{self, Memory};

/// Escape a user query for FTS5 MATCH safety.
/// Wraps each whitespace-separated token in double quotes so that
/// special characters like `-`, `/`, `.` are treated as literals
/// instead of FTS5 operators.
/// Escape and join tokens with OR for FTS5 MATCH.
/// OR semantics: match ANY token, rank sorts by how many matched.
fn sanitize_fts_query(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split_whitespace()
        .map(|token| {
            let escaped = token.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect();
    if tokens.len() <= 1 {
        tokens.join("")
    } else {
        tokens.join(" OR ")
    }
}

pub fn search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    _include_stale: bool,
) -> Result<Vec<Memory>> {
    search_with_branch(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        _include_stale,
        None,
    )
}

pub fn search_with_branch(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    _include_stale: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    let mut results = match query {
        Some(q) if !q.is_empty() => {
            // Keep original tokens for LIKE fallback (preserves short tokens like "DB")
            let orig_tokens: Vec<&str> = q.split_whitespace().collect();
            let orig_short: Vec<&str> = orig_tokens.iter().filter(|t| t.chars().count() < 3).copied().collect();
            // Expand query with synonyms (中英互查)
            let expanded = crate::query_expand::expand_query(q);
            let tokens: Vec<&str> = expanded.iter().map(|s| s.as_str()).collect();
            let long_tokens: Vec<&str> = tokens.iter().filter(|t| t.chars().count() >= 3).copied().collect();
            let short_tokens: Vec<&str> = orig_short; // Use original short tokens for LIKE

            let mut combined = Vec::new();
            let mut seen_ids = std::collections::HashSet::new();

            // FTS5 search with long tokens (OR semantics)
            if !long_tokens.is_empty() {
                let safe_query = sanitize_fts_query(&long_tokens.join(" "));
                let fts = memory::search_memories_fts(conn, &safe_query, project, memory_type, limit, 0)?;
                for m in fts {
                    seen_ids.insert(m.id);
                    combined.push(m);
                }
            }

            // LIKE fallback: always try with original tokens when FTS returns too few
            if (combined.len() as i64) < limit {
                let remaining = (limit - combined.len() as i64).max(1);
                let like = memory::search_memories_like(
                    conn, &orig_tokens, project, memory_type, remaining, 0,
                )?;
                for m in like {
                    if seen_ids.insert(m.id) {
                        combined.push(m);
                    }
                }
            }

            if combined.is_empty() {
                // Last resort: LIKE with expanded tokens
                memory::search_memories_like(conn, &tokens, project, memory_type, limit, 0)?
            } else {
                combined.truncate(limit as usize);
                combined
            }
        }
        _ => {
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                return Ok(vec![])
            } else if let Some(t) = memory_type {
                memory::get_memories_by_type(conn, proj, t, limit)?
            } else {
                memory::get_recent_memories(conn, proj, limit)?
            }
        }
    };

    // Post-filter by branch (NULL branch matches all — old data without branch)
    if let Some(br) = branch {
        results.retain(|m| match &m.branch {
            Some(b) => b == br,
            None => true, // old data without branch is visible everywhere
        });
    }

    Ok(results)
}

/// Search observations (used by get_observations MCP tool).
pub fn search_observations(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    obs_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<crate::db::Observation>> {
    use crate::db_models::OBSERVATION_TYPES;
    use crate::db_query;

    let mut results = match query {
        Some(q) if !q.is_empty() => {
            let tokens: Vec<&str> = q.split_whitespace().collect();
            let has_short_token = tokens.iter().any(|t| t.chars().count() < 3);

            if has_short_token {
                db_query::search_observations_like(
                    conn,
                    &tokens,
                    project,
                    obs_type,
                    limit,
                    offset,
                    include_stale,
                )?
            } else {
                let safe_query = sanitize_fts_query(q);
                db_query::search_observations_fts(
                    conn,
                    &safe_query,
                    project,
                    obs_type,
                    limit,
                    offset,
                    include_stale,
                )?
            }
        }
        _ => {
            let types: Vec<&str> = obs_type.map_or_else(|| OBSERVATION_TYPES.to_vec(), |t| vec![t]);
            let proj = project.unwrap_or("");
            if proj.is_empty() {
                return Ok(vec![]);
            } else {
                db_query::query_observations(conn, proj, &types, limit)?
            }
        }
    };

    // Observation project filter: push into SQL for FTS/LIKE paths too
    // For now, keep post-filter for observations since db_query functions
    // use a different project matching approach (push_project_filter).
    if let Some(proj) = project {
        results.retain(|o| {
            o.project
                .as_deref()
                .is_some_and(|p| p == proj || p.ends_with(&format!("/{proj}")))
        });
    }

    let start = offset as usize;
    if start >= results.len() {
        return Ok(vec![]);
    }
    let end = (start + limit as usize).min(results.len());
    Ok(results[start..end].to_vec())
}
