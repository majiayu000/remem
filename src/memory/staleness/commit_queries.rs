use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::path::{file_path_overlaps, parse_json_file_array};

pub(super) const SOURCE_COMMIT_FILES_SQL: &str = "
    SELECT c.id,
           COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch),
           c.branch,
           f.path
    FROM git_commit_sessions AS l
    CROSS JOIN git_commits AS c ON c.id = l.commit_id
    CROSS JOIN git_commit_files AS f ON f.commit_id = c.id
    WHERE c.project = ?1
      AND (l.memory_session_id = ?2 OR l.session_id = ?2)
      AND (?3 IS NULL OR c.branch = ?3 OR c.branch IS NULL)
      AND COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) <= ?4
    ORDER BY COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) DESC,
             c.id DESC";

pub(super) const SOURCE_COMMIT_JSON_SQL: &str = "
    SELECT c.id,
           COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch),
           c.branch,
           c.changed_files
    FROM git_commit_sessions AS l
    CROSS JOIN git_commits AS c ON c.id = l.commit_id
    WHERE c.project = ?1
      AND (l.memory_session_id = ?2 OR l.session_id = ?2)
      AND (?3 IS NULL OR c.branch = ?3 OR c.branch IS NULL)
      AND COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) <= ?4
    ORDER BY COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) DESC,
             c.id DESC";

pub(super) const LATER_COMMIT_FILES_SQL: &str = "
    SELECT f.path
    FROM git_commits AS c
    CROSS JOIN git_commit_files AS f ON f.commit_id = c.id
    WHERE c.project = ?1
      AND COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) > ?2
      AND (?4 IS NULL OR c.branch = ?4 OR c.branch IS NULL)
    UNION ALL
    SELECT f.path
    FROM git_commits AS c
    CROSS JOIN git_commit_files AS f ON f.commit_id = c.id
    WHERE c.project = ?1
      AND COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) = ?2
      AND c.id > ?3
      AND (?4 IS NULL OR c.branch = ?4 OR c.branch IS NULL)";

const LATER_COMMIT_JSON_SQL: &str = "
    SELECT changed_files
    FROM git_commits
    WHERE project = ?1
      AND COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) > ?2
      AND (?4 IS NULL OR branch = ?4 OR branch IS NULL)
    UNION ALL
    SELECT changed_files
    FROM git_commits
    WHERE project = ?1
      AND COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) = ?2
      AND id > ?3
      AND (?4 IS NULL OR branch = ?4 OR branch IS NULL)";

#[derive(Debug, Clone)]
pub(super) struct SourceAnchor {
    pub(super) id: i64,
    pub(super) epoch: i64,
    pub(super) branch: Option<String>,
}

pub(super) fn source_commit_anchor_for_file_sessions(
    conn: &Connection,
    project: &str,
    session_ids: &[String],
    branch_filter: Option<&str>,
    max_epoch: i64,
    touched_file: &str,
    use_commit_files: bool,
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
            use_commit_files,
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
    use_commit_files: bool,
) -> Result<Option<SourceAnchor>> {
    if use_commit_files {
        source_commit_anchor_from_files(
            conn,
            project,
            session_id,
            branch_filter,
            max_epoch,
            touched_file,
        )
    } else {
        source_commit_anchor_from_json(
            conn,
            project,
            session_id,
            branch_filter,
            max_epoch,
            touched_file,
        )
    }
}

fn source_commit_anchor_from_files(
    conn: &Connection,
    project: &str,
    session_id: &str,
    branch_filter: Option<&str>,
    max_epoch: i64,
    touched_file: &str,
) -> Result<Option<SourceAnchor>> {
    let mut statement = conn.prepare_cached(SOURCE_COMMIT_FILES_SQL)?;
    let rows = statement.query_map(
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
        let (anchor, changed_file) = row?;
        if file_path_overlaps(&changed_file, touched_file, project) {
            return Ok(Some(anchor));
        }
    }
    Ok(None)
}

fn source_commit_anchor_from_json(
    conn: &Connection,
    project: &str,
    session_id: &str,
    branch_filter: Option<&str>,
    max_epoch: i64,
    touched_file: &str,
) -> Result<Option<SourceAnchor>> {
    let mut statement = conn.prepare_cached(SOURCE_COMMIT_JSON_SQL)?;
    let rows = statement.query_map(
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
        let changed_files = parse_json_file_array(&raw_changed_files).with_context(|| {
            format!(
                "parse git commit {} changed_files for source-anchor staleness",
                anchor.id
            )
        })?;
        if changed_files
            .iter()
            .any(|changed_file| file_path_overlaps(changed_file, touched_file, project))
        {
            return Ok(Some(anchor));
        }
    }
    Ok(None)
}

pub(super) fn later_commit_touches_file(
    conn: &Connection,
    project: &str,
    anchor: &SourceAnchor,
    branch_filter: Option<&str>,
    touched_file: &str,
    use_commit_files: bool,
) -> Result<bool> {
    if use_commit_files {
        later_commit_file_relation_touches(conn, project, anchor, branch_filter, touched_file)
    } else {
        later_commit_json_touches(conn, project, anchor, branch_filter, touched_file)
    }
}

fn later_commit_file_relation_touches(
    conn: &Connection,
    project: &str,
    anchor: &SourceAnchor,
    branch_filter: Option<&str>,
    touched_file: &str,
) -> Result<bool> {
    let mut statement = conn.prepare_cached(LATER_COMMIT_FILES_SQL)?;
    let mut rows = statement.query(params![project, anchor.epoch, anchor.id, branch_filter])?;
    while let Some(row) = rows.next()? {
        let changed_file: String = row.get(0)?;
        if file_path_overlaps(&changed_file, touched_file, project) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn later_commit_json_touches(
    conn: &Connection,
    project: &str,
    anchor: &SourceAnchor,
    branch_filter: Option<&str>,
    touched_file: &str,
) -> Result<bool> {
    let mut statement = conn.prepare_cached(LATER_COMMIT_JSON_SQL)?;
    let mut rows = statement.query(params![project, anchor.epoch, anchor.id, branch_filter])?;
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

#[cfg(test)]
mod tests;
