use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{params, types::ToSql, Connection, OptionalExtension};
use serde::Serialize;

use super::Memory;
use capabilities::StalenessCapabilities;
use path::{file_path_overlaps, parse_file_list, parse_json_file_array};

mod capabilities;
mod path;

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
    session_row_ids: Vec<i64>,
    event_ids: Vec<i64>,
    file_epochs: HashMap<String, i64>,
}

#[derive(Debug, Default)]
struct CapturedEventRefs {
    session_ids: Vec<String>,
    session_row_ids: Vec<i64>,
    event_epochs: HashMap<i64, i64>,
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
    let capabilities = StalenessCapabilities::load(conn)?;
    let source_anchor = source_anchor_for_memory(conn, memory, &capabilities)?;
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
    if memories.is_empty() {
        return Ok(HashMap::new());
    }
    let mut labels = HashMap::new();
    let capabilities = StalenessCapabilities::load(conn)?;
    for memory in memories {
        let source_anchor = source_anchor_for_memory(conn, memory, &capabilities)?;
        labels.insert(
            memory.id,
            memory_staleness_label_for_anchor(memory, now_epoch, source_anchor),
        );
    }
    Ok(labels)
}

pub(crate) fn memory_staleness_labels_for_memories_lossy(
    conn: &Connection,
    memories: &[Memory],
    now_epoch: i64,
    mut on_error: impl FnMut(i64, &anyhow::Error),
) -> Result<HashMap<i64, MemoryStalenessLabel>> {
    if memories.is_empty() {
        return Ok(HashMap::new());
    }
    let mut labels = HashMap::new();
    let capabilities = StalenessCapabilities::load(conn)?;
    for memory in memories {
        let label = match source_anchor_for_memory(conn, memory, &capabilities) {
            Ok(source_anchor) => {
                memory_staleness_label_for_anchor(memory, now_epoch, source_anchor)
            }
            Err(error) => {
                on_error(memory.id, &error);
                memory_staleness_label(memory, now_epoch)
            }
        };
        labels.insert(memory.id, label);
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

fn source_anchor_for_memory(
    conn: &Connection,
    memory: &Memory,
    capabilities: &StalenessCapabilities,
) -> Result<&'static str> {
    if !capabilities.git_trace_tables_exist {
        return Ok("untracked");
    }

    let mut project = source_project_for_memory(conn, memory, capabilities)?;
    let mut session_ids = Vec::new();
    if let Some(session_id) = non_empty_trimmed(memory.session_id.as_deref()) {
        push_unique(&mut session_ids, session_id.to_string());
    }
    let mut file_epochs = file_epoch_map(
        parse_file_list(memory.files.as_deref(), &project)?,
        memory.updated_at_epoch,
    );
    if session_ids.is_empty() || file_epochs.is_empty() {
        let evidence = evidence_anchor_for_memory(conn, memory, &project, capabilities)?;
        project = evidence.project;
        for session_id in evidence.session_ids {
            push_unique(&mut session_ids, session_id);
        }
        for (file, epoch) in evidence.file_epochs {
            file_epochs.insert(file, epoch);
        }
    }
    if session_ids.is_empty() || file_epochs.is_empty() {
        return Ok("untracked");
    }

    let memory_branch = non_empty_trimmed(memory.branch.as_deref()).map(str::to_string);
    let mut anchored_any = false;
    for (file, max_epoch) in &file_epochs {
        let Some(anchor) = source_commit_anchor_for_file_sessions(
            conn,
            &project,
            &session_ids,
            memory_branch.as_deref(),
            *max_epoch,
            file,
        )?
        else {
            continue;
        };
        anchored_any = true;
        let branch_filter = memory_branch.as_deref().or(anchor.branch.as_deref());
        if later_commit_touches_file(conn, &project, &anchor, branch_filter, file)? {
            return Ok("verify-before-trust");
        }
    }
    if anchored_any {
        Ok("tracked")
    } else {
        Ok("untracked")
    }
}

fn source_commit_anchor_for_file_sessions(
    conn: &Connection,
    project: &str,
    session_ids: &[String],
    branch_filter: Option<&str>,
    max_epoch: i64,
    touched_file: &str,
) -> Result<Option<SourceAnchor>> {
    let mut latest = None;
    for session_id in session_ids {
        let Some(anchor) = source_commit_anchor_for_session(
            conn,
            project,
            session_id,
            branch_filter,
            max_epoch,
            touched_file,
        )?
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
    max_epoch: i64,
    touched_file: &str,
) -> Result<Option<SourceAnchor>> {
    let mut stmt = conn.prepare(
        "SELECT c.id,
                COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch),
                c.branch,
                c.changed_files
         FROM git_commits c
         JOIN git_commit_sessions l ON l.commit_id = c.id
         WHERE c.project = ?1
           AND (l.memory_session_id = ?2 OR l.session_id = ?2)
           AND (?3 IS NULL OR c.branch = ?3 OR c.branch IS NULL)
           AND COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) <= ?4
         ORDER BY COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) DESC,
                  c.id DESC
         ",
    )?;
    let rows = stmt.query_map(
        params![project, session_id, branch_filter, max_epoch],
        |row| {
            Ok((
                SourceAnchor {
                    id: row.get(0)?,
                    epoch: row.get(1)?,
                    branch: row.get(2)?,
                },
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    for row in rows {
        let (anchor, raw_changed_files) = row?;
        let changed_files = parse_json_file_array(&raw_changed_files)
            .with_context(|| "parse git commit changed_files for source-anchor staleness")?;
        if changed_files
            .iter()
            .any(|changed_file| file_path_overlaps(changed_file, touched_file, project))
        {
            return Ok(Some(anchor));
        }
    }
    Ok(None)
}

fn later_commit_touches_file(
    conn: &Connection,
    project: &str,
    anchor: &SourceAnchor,
    branch_filter: Option<&str>,
    touched_file: &str,
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
           AND (?4 IS NULL OR branch = ?4 OR branch IS NULL)",
    )?;
    let mut rows = stmt.query(params![project, anchor.epoch, anchor.id, branch_filter])?;
    while let Some(row) = rows.next()? {
        let raw: String = row.get(0)?;
        let changed_files = parse_json_file_array(&raw)
            .with_context(|| "parse git commit changed_files for source-anchor staleness")?;
        if changed_files
            .iter()
            .any(|changed_file| file_path_overlaps(changed_file, touched_file, project))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn source_project_for_memory(
    conn: &Connection,
    memory: &Memory,
    capabilities: &StalenessCapabilities,
) -> Result<String> {
    if !capabilities.memories_source_project {
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
    capabilities: &StalenessCapabilities,
) -> Result<EvidenceAnchor> {
    let (project, event_ids) =
        memory_evidence_reference(conn, memory, default_project, capabilities)?;
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
    anchor.event_ids = event_ids;
    let captured_refs =
        captured_event_refs(conn, &anchor.project, &anchor.event_ids, capabilities)?;
    anchor.session_ids = captured_refs.session_ids;
    anchor.session_row_ids = captured_refs.session_row_ids;
    let legacy_event_ids: Vec<i64> = anchor
        .event_ids
        .iter()
        .copied()
        .filter(|event_id| !captured_refs.event_epochs.contains_key(event_id))
        .collect();
    if !legacy_event_ids.is_empty() {
        add_legacy_event_files(
            conn,
            &anchor.project,
            &legacy_event_ids,
            &mut anchor.file_epochs,
            capabilities,
        )?;
    }
    add_observation_files(
        conn,
        &anchor.project,
        &anchor.event_ids,
        &anchor.session_row_ids,
        &anchor.session_ids,
        &captured_refs.event_epochs,
        &mut anchor.file_epochs,
        capabilities,
    )?;
    Ok(anchor)
}

fn memory_evidence_reference(
    conn: &Connection,
    memory: &Memory,
    default_project: &str,
    capabilities: &StalenessCapabilities,
) -> Result<(String, Vec<i64>)> {
    if !capabilities.memories_exists || !capabilities.memories_evidence_event_ids {
        return Ok((default_project.to_string(), Vec::new()));
    }
    let (project, memory_evidence_json, source_candidate_id) = match (
        capabilities.memories_source_candidate_id,
        capabilities.memories_source_project,
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
    candidate_evidence_reference(conn, source_candidate_id, &project, capabilities)
}

fn candidate_evidence_reference(
    conn: &Connection,
    candidate_id: i64,
    default_project: &str,
    capabilities: &StalenessCapabilities,
) -> Result<(String, Vec<i64>)> {
    if !capabilities.memory_candidates_exists || !capabilities.memory_candidates_evidence_event_ids
    {
        return Ok((default_project.to_string(), Vec::new()));
    }
    let row = if capabilities.projects_exists {
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

fn captured_event_refs(
    conn: &Connection,
    project: &str,
    event_ids: &[i64],
    capabilities: &StalenessCapabilities,
) -> Result<CapturedEventRefs> {
    if event_ids.is_empty() || !capabilities.captured_events_exists || !capabilities.projects_exists
    {
        return Ok(CapturedEventRefs::default());
    }
    let placeholders = placeholders(2, event_ids.len());
    let event_time_expr = if capabilities.captured_events_reference_time_epoch {
        "COALESCE(e.reference_time_epoch, e.created_at_epoch)"
    } else {
        "e.created_at_epoch"
    };
    let sql = format!(
        "SELECT e.id, e.session_id, e.session_row_id, {event_time_expr}
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
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut refs = CapturedEventRefs::default();
    for row in rows {
        let (event_id, session_id, session_row_id, created_at_epoch) = row?;
        push_unique(&mut refs.session_ids, session_id);
        push_unique_i64(&mut refs.session_row_ids, session_row_id);
        refs.event_epochs.insert(event_id, created_at_epoch);
    }
    Ok(refs)
}

fn add_legacy_event_files(
    conn: &Connection,
    project: &str,
    event_ids: &[i64],
    file_epochs: &mut HashMap<String, i64>,
    capabilities: &StalenessCapabilities,
) -> Result<()> {
    if event_ids.is_empty() || !capabilities.events_exists {
        return Ok(());
    }
    let placeholders = placeholders(2, event_ids.len());
    let sql = format!(
        "SELECT files, created_at_epoch
         FROM events
         WHERE project = ?1
           AND id IN ({placeholders})
           AND files IS NOT NULL"
    );
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(project.to_string())];
    for event_id in event_ids {
        values.push(Box::new(*event_id));
    }
    let refs = crate::db::to_sql_refs(&values);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (files, created_at_epoch) = row?;
        add_file_epochs(Some(&files), project, created_at_epoch, file_epochs)?;
    }
    Ok(())
}

fn add_observation_files(
    conn: &Connection,
    project: &str,
    event_ids: &[i64],
    session_row_ids: &[i64],
    session_ids: &[String],
    event_epochs: &HashMap<i64, i64>,
    file_epochs: &mut HashMap<String, i64>,
    capabilities: &StalenessCapabilities,
) -> Result<()> {
    if event_ids.is_empty() || !capabilities.observations_exists {
        return Ok(());
    }
    if capabilities.observations_session_row_id && capabilities.observations_evidence_event_ids {
        return add_observation_files_by_evidence_events(
            conn,
            project,
            event_ids,
            session_row_ids,
            event_epochs,
            file_epochs,
        );
    }
    add_legacy_observation_files(conn, project, session_ids, file_epochs)
}

fn add_observation_files_by_evidence_events(
    conn: &Connection,
    project: &str,
    event_ids: &[i64],
    session_row_ids: &[i64],
    event_epochs: &HashMap<i64, i64>,
    file_epochs: &mut HashMap<String, i64>,
) -> Result<()> {
    if session_row_ids.is_empty() {
        return Ok(());
    }
    let wanted = event_ids.iter().copied().collect::<HashSet<_>>();
    let placeholders = placeholders(2, session_row_ids.len());
    let sql = format!(
        "SELECT files_read, files_modified, evidence_event_ids, created_at_epoch
         FROM observations
         WHERE project = ?1
           AND session_row_id IN ({placeholders})
           AND evidence_event_ids IS NOT NULL
           AND (
             files_read IS NOT NULL
             OR files_modified IS NOT NULL
           )"
    );
    let mut values: Vec<Box<dyn ToSql>> = vec![Box::new(project.to_string())];
    for session_row_id in session_row_ids {
        values.push(Box::new(*session_row_id));
    }
    let refs = crate::db::to_sql_refs(&values);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    for row in rows {
        let (files_read, files_modified, evidence_json, created_at_epoch) = row?;
        let observation_event_ids =
            parse_evidence_event_ids(evidence_json.as_deref(), "observation evidence_event_ids")?;
        if observation_event_ids
            .iter()
            .any(|event_id| wanted.contains(event_id))
        {
            let source_epoch = observation_event_ids
                .iter()
                .filter(|event_id| wanted.contains(event_id))
                .filter_map(|event_id| event_epochs.get(event_id))
                .max()
                .copied()
                .unwrap_or(created_at_epoch);
            add_file_epochs(files_read.as_deref(), project, source_epoch, file_epochs)?;
            add_file_epochs(
                files_modified.as_deref(),
                project,
                source_epoch,
                file_epochs,
            )?;
        }
    }
    Ok(())
}

fn add_legacy_observation_files(
    conn: &Connection,
    project: &str,
    session_ids: &[String],
    file_epochs: &mut HashMap<String, i64>,
) -> Result<()> {
    if session_ids.is_empty() {
        return Ok(());
    }
    let placeholders = placeholders(2, session_ids.len());
    let sql = format!(
        "SELECT files_read, files_modified, created_at_epoch
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
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (files_read, files_modified, created_at_epoch) = row?;
        add_file_epochs(
            files_read.as_deref(),
            project,
            created_at_epoch,
            file_epochs,
        )?;
        add_file_epochs(
            files_modified.as_deref(),
            project,
            created_at_epoch,
            file_epochs,
        )?;
    }
    Ok(())
}

fn file_epoch_map(files: HashSet<String>, epoch: i64) -> HashMap<String, i64> {
    files.into_iter().map(|file| (file, epoch)).collect()
}

fn add_file_epochs(
    raw_files: Option<&str>,
    project: &str,
    epoch: i64,
    file_epochs: &mut HashMap<String, i64>,
) -> Result<()> {
    merge_file_epochs_max(
        file_epochs,
        file_epoch_map(parse_file_list(raw_files, project)?, epoch),
    );
    Ok(())
}

fn merge_file_epochs_max(target: &mut HashMap<String, i64>, source: HashMap<String, i64>) {
    for (file, epoch) in source {
        target
            .entry(file)
            .and_modify(|existing| *existing = (*existing).max(epoch))
            .or_insert(epoch);
    }
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

fn push_unique_i64(values: &mut Vec<i64>, value: i64) {
    if !values.contains(&value) {
        values.push(value);
    }
}

#[cfg(test)]
mod tests;
