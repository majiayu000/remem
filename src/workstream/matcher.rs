use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::{
    identity::{
        has_continuity_alias, normalize_title, title_has_continuity, MATCH_REASON_ALIAS_EXACT,
        MATCH_REASON_SESSION_LINK, MATCH_REASON_TITLE_CONTAINS, MATCH_REASON_TITLE_EXACT,
    },
    query::map_workstream_row,
    WorkStream,
};

pub(super) struct WorkStreamMatch {
    pub workstream: WorkStream,
    pub reason: &'static str,
}

pub fn find_matching_workstream(
    conn: &Connection,
    project: &str,
    title: &str,
) -> Result<Option<WorkStream>> {
    Ok(find_title_workstream(conn, project, title)?.map(|matched| matched.workstream))
}

pub(super) fn find_workstream_for_upsert(
    conn: &Connection,
    project: &str,
    memory_session_id: &str,
    title: &str,
) -> Result<Option<WorkStreamMatch>> {
    if let Some(matched) = find_linked_workstream(conn, project, memory_session_id, title)? {
        return Ok(Some(matched));
    }
    if let Some(matched) = find_alias_workstream(conn, project, title)? {
        return Ok(Some(matched));
    }
    find_title_workstream(conn, project, title)
}

fn find_title_workstream(
    conn: &Connection,
    project: &str,
    title: &str,
) -> Result<Option<WorkStreamMatch>> {
    let exact = conn
        .query_row(
            "SELECT id, project, title, description, status, progress, next_action, blockers,
                    created_at_epoch, updated_at_epoch, completed_at_epoch
             FROM workstreams
             WHERE title = ?2 AND status IN ('active', 'paused')
               AND merged_into_workstream_id IS NULL
               AND ((owner_scope = 'repo' AND owner_key = ?1)
                    OR (owner_scope = 'repo' AND target_project = ?1)
                    OR (owner_scope = 'workstream' AND target_project = ?1)
                    OR (owner_scope IS NULL AND project = ?1))",
            params![project, title],
            map_workstream_row,
        )
        .optional()?;
    if let Some(workstream) = exact {
        return Ok(Some(WorkStreamMatch {
            workstream,
            reason: MATCH_REASON_TITLE_EXACT,
        }));
    }

    let title_lower = title.to_lowercase();
    let mut stmt = conn.prepare(
        "SELECT id, project, title, description, status, progress, next_action, blockers,
                created_at_epoch, updated_at_epoch, completed_at_epoch
         FROM workstreams
         WHERE status IN ('active', 'paused')
           AND merged_into_workstream_id IS NULL
           AND ((owner_scope = 'repo' AND owner_key = ?1)
                OR (owner_scope = 'repo' AND target_project = ?1)
                OR (owner_scope = 'workstream' AND target_project = ?1)
                OR (owner_scope IS NULL AND project = ?1))
         ORDER BY updated_at_epoch DESC",
    )?;
    let rows = stmt.query_map(params![project], map_workstream_row)?;
    for row in rows {
        let workstream = row?;
        let candidate_title = workstream.title.to_lowercase();
        if candidate_title.contains(&title_lower) || title_lower.contains(&candidate_title) {
            return Ok(Some(WorkStreamMatch {
                workstream,
                reason: MATCH_REASON_TITLE_CONTAINS,
            }));
        }
    }

    Ok(None)
}

fn find_linked_workstream(
    conn: &Connection,
    project: &str,
    memory_session_id: &str,
    title: &str,
) -> Result<Option<WorkStreamMatch>> {
    if !memory_session_id_maps_to_unique_content_session(conn, memory_session_id)? {
        crate::log::warn(
            "workstream",
            &format!("session_link_collision project={project} session={memory_session_id}"),
        );
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT DISTINCT ws.id, ws.project, ws.title, ws.description, ws.status, ws.progress,
                ws.next_action, ws.blockers, ws.created_at_epoch, ws.updated_at_epoch,
                ws.completed_at_epoch
         FROM workstreams ws
         JOIN workstream_sessions wss ON wss.workstream_id = ws.id
         WHERE wss.memory_session_id = ?2
           AND ws.status IN ('active', 'paused')
           AND ws.merged_into_workstream_id IS NULL
           AND ((ws.owner_scope = 'repo' AND ws.owner_key = ?1)
                OR (ws.owner_scope = 'repo' AND ws.target_project = ?1)
                OR (ws.owner_scope = 'workstream' AND ws.target_project = ?1)
                OR (ws.owner_scope IS NULL AND ws.project = ?1))
         ORDER BY ws.updated_at_epoch DESC",
    )?;
    let rows = stmt.query_map(params![project, memory_session_id], map_workstream_row)?;
    let candidates = crate::db::query::collect_rows(rows)?;

    let mut continuity_candidates = Vec::new();
    for candidate in &candidates {
        if title_has_continuity(&candidate.title, title)
            || has_continuity_alias(conn, candidate.id, title)?
        {
            continuity_candidates.push(candidate);
        }
    }

    let [candidate] = continuity_candidates.as_slice() else {
        if continuity_candidates.len() > 1 {
            crate::log::warn(
                "workstream",
                &format!(
                    "session_link_ambiguous project={project} session={memory_session_id} candidates={}",
                    continuity_candidates.len()
                ),
            );
        } else if !candidates.is_empty() {
            crate::log::warn(
                "workstream",
                &format!(
                    "session_link_without_continuity project={project} session={memory_session_id} candidates={}",
                    candidates.len()
                ),
            );
        }
        return Ok(None);
    };

    Ok(Some(WorkStreamMatch {
        workstream: (*candidate).clone(),
        reason: MATCH_REASON_SESSION_LINK,
    }))
}

fn memory_session_id_maps_to_unique_content_session(
    conn: &Connection,
    memory_session_id: &str,
) -> Result<bool> {
    if !crate::retrieval::temporal::sqlite_table_exists(conn, "sdk_sessions")? {
        return Ok(true);
    }

    let content_session_count: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT content_session_id)
         FROM sdk_sessions
         WHERE memory_session_id = ?1",
        params![memory_session_id],
        |row| row.get(0),
    )?;
    Ok(content_session_count <= 1)
}

fn find_alias_workstream(
    conn: &Connection,
    project: &str,
    title: &str,
) -> Result<Option<WorkStreamMatch>> {
    let normalized_title = normalize_title(title);
    if normalized_title.is_empty() {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT ws.id, ws.project, ws.title, ws.description, ws.status, ws.progress,
                ws.next_action, ws.blockers, ws.created_at_epoch, ws.updated_at_epoch,
                ws.completed_at_epoch
         FROM workstream_aliases wa
         JOIN workstreams ws ON ws.id = wa.workstream_id
         WHERE wa.normalized_title = ?2
           AND ws.status IN ('active', 'paused')
           AND ws.merged_into_workstream_id IS NULL
           AND ((ws.owner_scope = 'repo' AND ws.owner_key = ?1)
                OR (ws.owner_scope = 'repo' AND ws.target_project = ?1)
                OR (ws.owner_scope = 'workstream' AND ws.target_project = ?1)
                OR (ws.owner_scope IS NULL AND ws.project = ?1))
         ORDER BY ws.updated_at_epoch DESC",
    )?;
    let rows = stmt.query_map(params![project, normalized_title], map_workstream_row)?;
    let candidates = crate::db::query::collect_rows(rows)?;

    let [candidate] = candidates.as_slice() else {
        if candidates.len() > 1 {
            crate::log::warn(
                "workstream",
                &format!(
                    "alias_exact_ambiguous project={project} normalized_title={normalized_title} candidates={}",
                    candidates.len()
                ),
            );
        }
        return Ok(None);
    };

    Ok(Some(WorkStreamMatch {
        workstream: candidate.clone(),
        reason: MATCH_REASON_ALIAS_EXACT,
    }))
}
