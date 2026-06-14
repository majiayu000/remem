use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{params, types::ToSql, Connection, OptionalExtension};
use serde::Serialize;

use super::Memory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryStalenessLabel {
    pub status: String,
    pub age: &'static str,
    pub source_anchor: String,
    pub label: String,
}

#[derive(Debug, Clone)]
struct SourceAnchor {
    id: i64,
    epoch: i64,
    branch: Option<String>,
}

#[derive(Debug, Default)]
struct EvidenceAnchor {
    project: String,
    session_ids: Vec<String>,
    touched_files: HashSet<String>,
}

pub fn memory_staleness_label(memory: &Memory, now_epoch: i64) -> MemoryStalenessLabel {
    memory_staleness_label_for_anchor(memory, now_epoch, "untracked")
}

pub fn memory_staleness_label_for_anchor(
    memory: &Memory,
    now_epoch: i64,
    source_anchor: impl Into<String>,
) -> MemoryStalenessLabel {
    let age = age_staleness(memory.updated_at_epoch, now_epoch);
    let source_anchor = source_anchor.into();
    MemoryStalenessLabel {
        status: memory.status.clone(),
        age,
        source_anchor: source_anchor.clone(),
        label: format!(
            "status={}; staleness={age}; source_anchor={source_anchor}",
            memory.status
        ),
    }
}

pub fn memory_staleness_label_with_conn(
    conn: &Connection,
    memory: &Memory,
    now_epoch: i64,
) -> Result<MemoryStalenessLabel> {
    let source_anchor = source_anchor_for_memory(conn, memory)?;
    Ok(memory_staleness_label_for_anchor(
        memory,
        now_epoch,
        source_anchor,
    ))
}

pub fn memory_staleness_labels_for_memories(
    conn: &Connection,
    memories: &[Memory],
    now_epoch: i64,
) -> Result<HashMap<i64, MemoryStalenessLabel>> {
    let mut labels = HashMap::new();
    for memory in memories {
        labels.insert(
            memory.id,
            memory_staleness_label_with_conn(conn, memory, now_epoch)?,
        );
    }
    Ok(labels)
}

pub fn memory_staleness(memory: &Memory, now_epoch: i64) -> String {
    memory_staleness_label(memory, now_epoch).label
}

pub fn age_staleness_label(updated_at_epoch: i64, now_epoch: i64) -> String {
    format!("staleness={}", age_staleness(updated_at_epoch, now_epoch))
}

pub fn age_staleness(updated_at_epoch: i64, now_epoch: i64) -> &'static str {
    let age_days = now_epoch.saturating_sub(updated_at_epoch) / 86_400;
    if age_days <= 30 {
        "fresh"
    } else if age_days <= 90 {
        "aging"
    } else {
        "old"
    }
}

fn source_anchor_for_memory(conn: &Connection, memory: &Memory) -> Result<&'static str> {
    if !git_trace_tables_exist(conn)? {
        return Ok("untracked");
    }

    let mut project = source_project_for_memory(conn, memory)?;
    let mut session_ids = Vec::new();
    if let Some(session_id) = non_empty_trimmed(memory.session_id.as_deref()) {
        push_unique(&mut session_ids, session_id.to_string());
    }
    let mut touched_files = parse_file_list(memory.files.as_deref())?;
    if session_ids.is_empty() || touched_files.is_empty() {
        let evidence = evidence_anchor_for_memory(conn, memory, &project)?;
        project = evidence.project;
        for session_id in evidence.session_ids {
            push_unique(&mut session_ids, session_id);
        }
        touched_files.extend(evidence.touched_files);
    }
    if session_ids.is_empty() || touched_files.is_empty() {
        return Ok("untracked");
    }

    let memory_branch = non_empty_trimmed(memory.branch.as_deref()).map(str::to_string);
    let Some(anchor) =
        source_commit_anchor_for_sessions(conn, &project, &session_ids, memory_branch.as_deref())?
    else {
        return Ok("untracked");
    };
    let branch_filter = memory_branch.or_else(|| anchor.branch.clone());
    if later_commit_touches_any_file(
        conn,
        &project,
        &anchor,
        branch_filter.as_deref(),
        &touched_files,
    )? {
        Ok("verify-before-trust")
    } else {
        Ok("tracked")
    }
}

fn git_trace_tables_exist(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM sqlite_master
         WHERE type = 'table'
           AND name IN ('git_commits', 'git_commit_sessions')",
        [],
        |row| row.get(0),
    )?;
    Ok(count == 2)
}

fn source_commit_anchor_for_sessions(
    conn: &Connection,
    project: &str,
    session_ids: &[String],
    branch_filter: Option<&str>,
) -> Result<Option<SourceAnchor>> {
    let mut latest = None;
    for session_id in session_ids {
        let Some(anchor) =
            source_commit_anchor_for_session(conn, project, session_id, branch_filter)?
        else {
            continue;
        };
        if latest.as_ref().is_none_or(|current: &SourceAnchor| {
            (anchor.epoch, anchor.id) > (current.epoch, current.id)
        }) {
            latest = Some(anchor);
        }
    }
    Ok(latest)
}

fn source_commit_anchor_for_session(
    conn: &Connection,
    project: &str,
    session_id: &str,
    branch_filter: Option<&str>,
) -> Result<Option<SourceAnchor>> {
    conn.query_row(
        "SELECT c.id, COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch), c.branch
         FROM git_commits c
         JOIN git_commit_sessions l ON l.commit_id = c.id
         WHERE c.project = ?1
           AND (l.memory_session_id = ?2 OR l.session_id = ?2)
           AND (?3 IS NULL OR c.branch = ?3)
         ORDER BY COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) DESC,
                  c.id DESC
         LIMIT 1",
        params![project, session_id, branch_filter],
        |row| {
            Ok(SourceAnchor {
                id: row.get(0)?,
                epoch: row.get(1)?,
                branch: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn later_commit_touches_any_file(
    conn: &Connection,
    project: &str,
    anchor: &SourceAnchor,
    branch_filter: Option<&str>,
    touched_files: &HashSet<String>,
) -> Result<bool> {
    let mut stmt = conn.prepare(
        "SELECT changed_files
         FROM git_commits
         WHERE project = ?1
           AND (
             COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) > ?2
             OR (
               COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) = ?2
               AND id > ?3
             )
           )
           AND (?4 IS NULL OR branch = ?4)",
    )?;
    let mut rows = stmt.query(params![project, anchor.epoch, anchor.id, branch_filter])?;
    while let Some(row) = rows.next()? {
        let raw: String = row.get(0)?;
        let changed_files = parse_json_file_array(&raw)
            .with_context(|| "parse git commit changed_files for source-anchor staleness")?;
        if changed_files
            .iter()
            .any(|changed_file| paths_overlap(changed_file, touched_files))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn source_project_for_memory(conn: &Connection, memory: &Memory) -> Result<String> {
    if !column_exists(conn, "memories", "source_project")? {
        return Ok(memory.project.clone());
    }
    Ok(conn
        .query_row(
            "SELECT COALESCE(source_project, project) FROM memories WHERE id = ?1",
            [memory.id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| memory.project.clone()))
}

fn evidence_anchor_for_memory(
    conn: &Connection,
    memory: &Memory,
    default_project: &str,
) -> Result<EvidenceAnchor> {
    let (project, event_ids) = memory_evidence_reference(conn, memory, default_project)?;
    if event_ids.is_empty() {
        return Ok(EvidenceAnchor {
            project,
            ..Default::default()
        });
    }
    let mut anchor = EvidenceAnchor {
        project,
        ..Default::default()
    };
    anchor.session_ids = captured_event_sessions(conn, &anchor.project, &event_ids)?;
    add_legacy_event_files(
        conn,
        &anchor.project,
        &anchor.session_ids,
        &mut anchor.touched_files,
    )?;
    add_observation_files(
        conn,
        &anchor.project,
        &anchor.session_ids,
        &mut anchor.touched_files,
    )?;
    Ok(anchor)
}

fn memory_evidence_reference(
    conn: &Connection,
    memory: &Memory,
    default_project: &str,
) -> Result<(String, Vec<i64>)> {
    if !table_exists(conn, "memories")? || !column_exists(conn, "memories", "evidence_event_ids")? {
        return Ok((default_project.to_string(), Vec::new()));
    }
    let has_source_candidate = column_exists(conn, "memories", "source_candidate_id")?;
    let has_source_project = column_exists(conn, "memories", "source_project")?;
    let (project, memory_evidence_json, source_candidate_id) = match (
        has_source_candidate,
        has_source_project,
    ) {
        (true, true) => conn
            .query_row(
                "SELECT COALESCE(source_project, project), evidence_event_ids, source_candidate_id
             FROM memories WHERE id = ?1",
                [memory.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?
            .unwrap_or_else(|| (default_project.to_string(), None, None)),
        (true, false) => conn
            .query_row(
                "SELECT project, evidence_event_ids, source_candidate_id
                 FROM memories WHERE id = ?1",
                [memory.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()?
            .unwrap_or_else(|| (default_project.to_string(), None, None)),
        (false, true) => conn
            .query_row(
                "SELECT COALESCE(source_project, project), evidence_event_ids
                 FROM memories WHERE id = ?1",
                [memory.id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        None,
                    ))
                },
            )
            .optional()?
            .unwrap_or_else(|| (default_project.to_string(), None, None)),
        (false, false) => {
            let evidence_json = conn
                .query_row(
                    "SELECT evidence_event_ids FROM memories WHERE id = ?1",
                    [memory.id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
            (default_project.to_string(), evidence_json, None)
        }
    };

    let memory_event_ids = parse_evidence_event_ids(
        memory_evidence_json.as_deref(),
        &format!("memory {} evidence_event_ids", memory.id),
    )?;
    if !memory_event_ids.is_empty() {
        return Ok((project, memory_event_ids));
    }
    let Some(source_candidate_id) = source_candidate_id else {
        return Ok((project, Vec::new()));
    };
    candidate_evidence_reference(conn, source_candidate_id, &project)
}

fn candidate_evidence_reference(
    conn: &Connection,
    candidate_id: i64,
    default_project: &str,
) -> Result<(String, Vec<i64>)> {
    if !table_exists(conn, "memory_candidates")?
        || !column_exists(conn, "memory_candidates", "evidence_event_ids")?
    {
        return Ok((default_project.to_string(), Vec::new()));
    }
    let row = if table_exists(conn, "projects")? {
        conn.query_row(
            "SELECT COALESCE(p.project_path, ?2), c.evidence_event_ids
             FROM memory_candidates c
             LEFT JOIN projects p ON p.id = c.project_id
             WHERE c.id = ?1",
            params![candidate_id, default_project],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    } else {
        conn.query_row(
            "SELECT ?2, evidence_event_ids
             FROM memory_candidates
             WHERE id = ?1",
            params![candidate_id, default_project],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
    };
    let Some((project, evidence_json)) = row else {
        return Ok((default_project.to_string(), Vec::new()));
    };
    let event_ids = parse_evidence_event_ids(
        Some(&evidence_json),
        &format!("memory candidate {candidate_id} evidence_event_ids"),
    )?;
    Ok((project, event_ids))
}

fn captured_event_sessions(
    conn: &Connection,
    project: &str,
    event_ids: &[i64],
) -> Result<Vec<String>> {
    if event_ids.is_empty()
        || !table_exists(conn, "captured_events")?
        || !table_exists(conn, "projects")?
    {
        return Ok(Vec::new());
    }
    let placeholders = placeholders(2, event_ids.len());
    let sql = format!(
        "SELECT DISTINCT e.session_id
         FROM captured_events e
         JOIN projects p ON p.id = e.project_id
         WHERE p.project_path = ?1
           AND e.id IN ({placeholders})
         ORDER BY e.id ASC"
    );
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(project.to_string())];
    for event_id in event_ids {
        values.push(Box::new(*event_id));
    }
    let refs = crate::db::to_sql_refs(&values);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, String>(0))?;
    let mut session_ids = Vec::new();
    for row in rows {
        push_unique(&mut session_ids, row?);
    }
    Ok(session_ids)
}

fn add_legacy_event_files(
    conn: &Connection,
    project: &str,
    session_ids: &[String],
    touched_files: &mut HashSet<String>,
) -> Result<()> {
    if session_ids.is_empty() || !table_exists(conn, "events")? {
        return Ok(());
    }
    let placeholders = placeholders(2, session_ids.len());
    let sql = format!(
        "SELECT files
         FROM events
         WHERE project = ?1
           AND session_id IN ({placeholders})
           AND files IS NOT NULL"
    );
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(project.to_string())];
    for session_id in session_ids {
        values.push(Box::new(session_id.clone()));
    }
    let refs = crate::db::to_sql_refs(&values);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, String>(0))?;
    for row in rows {
        touched_files.extend(parse_file_list(Some(&row?))?);
    }
    Ok(())
}

fn add_observation_files(
    conn: &Connection,
    project: &str,
    session_ids: &[String],
    touched_files: &mut HashSet<String>,
) -> Result<()> {
    if session_ids.is_empty() || !table_exists(conn, "observations")? {
        return Ok(());
    }
    let placeholders = placeholders(2, session_ids.len());
    let sql = format!(
        "SELECT files_read, files_modified
         FROM observations
         WHERE project = ?1
           AND memory_session_id IN ({placeholders})
           AND (
             files_read IS NOT NULL
             OR files_modified IS NOT NULL
           )"
    );
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(project.to_string())];
    for session_id in session_ids {
        values.push(Box::new(session_id.clone()));
    }
    let refs = crate::db::to_sql_refs(&values);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
        ))
    })?;
    for row in rows {
        let (files_read, files_modified) = row?;
        touched_files.extend(parse_file_list(files_read.as_deref())?);
        touched_files.extend(parse_file_list(files_modified.as_deref())?);
    }
    Ok(())
}

fn parse_evidence_event_ids(raw: Option<&str>, context: &str) -> Result<Vec<i64>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut ids = serde_json::from_str::<Vec<i64>>(raw)
        .with_context(|| format!("parse {context} for source-anchor staleness"))?;
    ids.retain(|id| *id > 0);
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    let exists = conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_master
             WHERE type = 'table' AND name = ?1
         )",
        [table],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(exists != 0)
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn placeholders(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn non_empty_trimmed(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn parse_file_list(raw: Option<&str>) -> Result<HashSet<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(HashSet::new());
    };
    let files = if raw.starts_with('[') {
        parse_json_file_array(raw)
            .with_context(|| "parse memory files for source-anchor staleness")?
    } else {
        raw.split([',', '\n'])
            .map(str::trim)
            .map(|value| value.trim_matches('"'))
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    };
    Ok(files
        .into_iter()
        .filter_map(|file| normalize_file_path(&file))
        .collect())
}

fn parse_json_file_array(raw: &str) -> Result<Vec<String>> {
    let files = serde_json::from_str::<Vec<String>>(raw)?;
    Ok(files)
}

fn paths_overlap(changed_file: &str, touched_files: &HashSet<String>) -> bool {
    let Some(changed_file) = normalize_file_path(changed_file) else {
        return false;
    };
    touched_files.iter().any(|memory_file| {
        changed_file == *memory_file
            || changed_file
                .strip_prefix(memory_file)
                .is_some_and(|tail| tail.starts_with('/'))
            || memory_file
                .strip_prefix(&changed_file)
                .is_some_and(|tail| tail.starts_with('/'))
    })
}

fn normalize_file_path(path: &str) -> Option<String> {
    let trimmed = path.trim().trim_start_matches("./").trim_matches('/');
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests;
