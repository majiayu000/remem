use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::Memory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MemoryStalenessLabel {
    pub status: String,
    pub age: &'static str,
    pub source_anchor: String,
    pub label: String,
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
    let Some(session_id) = memory
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok("untracked");
    };
    let touched_files = parse_file_list(memory.files.as_deref())?;
    if touched_files.is_empty() || !git_trace_tables_exist(conn)? {
        return Ok("untracked");
    }
    let Some(anchor) = source_commit_anchor(conn, &memory.project, session_id)? else {
        return Ok("untracked");
    };
    if later_commit_touches_any_file(conn, &memory.project, anchor, &touched_files)? {
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

fn source_commit_anchor(
    conn: &Connection,
    project: &str,
    session_id: &str,
) -> Result<Option<(i64, i64)>> {
    conn.query_row(
        "SELECT c.id, COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch)
         FROM git_commits c
         JOIN git_commit_sessions l ON l.commit_id = c.id
         WHERE c.project = ?1
           AND (l.memory_session_id = ?2 OR l.session_id = ?2)
         ORDER BY COALESCE(c.authored_at_epoch, c.updated_at_epoch, c.created_at_epoch) DESC,
                  c.id DESC
         LIMIT 1",
        params![project, session_id],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

fn later_commit_touches_any_file(
    conn: &Connection,
    project: &str,
    anchor: (i64, i64),
    touched_files: &HashSet<String>,
) -> Result<bool> {
    let (anchor_id, anchor_epoch) = anchor;
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
           )",
    )?;
    let mut rows = stmt.query(params![project, anchor_epoch, anchor_id])?;
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
mod tests {
    use super::*;

    fn memory(updated_at_epoch: i64, status: &str) -> Memory {
        Memory {
            id: 1,
            session_id: None,
            project: "/repo".to_string(),
            topic_key: None,
            title: "Staleness fixture".to_string(),
            text: "body".to_string(),
            memory_type: "decision".to_string(),
            files: None,
            created_at_epoch: updated_at_epoch,
            updated_at_epoch,
            status: status.to_string(),
            branch: None,
            scope: "project".to_string(),
        }
    }

    #[test]
    fn labels_memory_status_age_and_untracked_source_anchor() {
        let label = memory_staleness_label(&memory(1_700_000_000, "active"), 1_700_000_000);

        assert_eq!(label.status, "active");
        assert_eq!(label.age, "fresh");
        assert_eq!(label.source_anchor, "untracked");
        assert_eq!(
            label.label,
            "status=active; staleness=fresh; source_anchor=untracked"
        );
        assert_eq!(
            memory_staleness(&memory(1_700_000_000, "active"), 1_700_000_000),
            "status=active; staleness=fresh; source_anchor=untracked"
        );
    }

    #[test]
    fn classifies_age_buckets() {
        let now = 1_700_000_000;

        assert_eq!(age_staleness(now - 30 * 86_400, now), "fresh");
        assert_eq!(age_staleness(now - 31 * 86_400, now), "aging");
        assert_eq!(age_staleness(now - 91 * 86_400, now), "old");
    }

    #[test]
    fn source_anchor_marks_untracked_without_files_or_commit_link() -> Result<()> {
        let conn = migrated_db()?;
        let mut memory = memory(1_700_000_000, "active");
        memory.session_id = Some("mem-session-1".to_string());

        let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

        assert_eq!(label.source_anchor, "untracked");
        Ok(())
    }

    #[test]
    fn source_anchor_tracks_commit_without_later_file_change() -> Result<()> {
        let conn = migrated_db()?;
        let memory = tracked_memory(Some(r#"["src/lib.rs"]"#));
        link_commit(
            &conn,
            1,
            "source-sha",
            100,
            &["src/lib.rs"],
            "mem-session-1",
        )?;
        insert_commit(&conn, 2, "later-sha", 200, &["README.md"])?;

        let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

        assert_eq!(label.source_anchor, "tracked");
        assert!(label.label.contains("source_anchor=tracked"));
        Ok(())
    }

    #[test]
    fn source_anchor_requires_verification_after_later_file_change() -> Result<()> {
        let conn = migrated_db()?;
        let memory = tracked_memory(Some(r#"["src/lib.rs"]"#));
        link_commit(
            &conn,
            1,
            "source-sha",
            100,
            &["src/lib.rs"],
            "mem-session-1",
        )?;
        insert_commit(&conn, 2, "later-sha", 200, &["src/lib.rs"])?;

        let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

        assert_eq!(label.source_anchor, "verify-before-trust");
        assert!(label.label.contains("source_anchor=verify-before-trust"));
        Ok(())
    }

    #[test]
    fn source_anchor_matches_directory_overlap() -> Result<()> {
        let conn = migrated_db()?;
        let memory = tracked_memory(Some(r#"["src/context"]"#));
        link_commit(
            &conn,
            1,
            "source-sha",
            100,
            &["src/context/query.rs"],
            "mem-session-1",
        )?;
        insert_commit(&conn, 2, "later-sha", 200, &["src/context/render.rs"])?;

        let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

        assert_eq!(label.source_anchor, "verify-before-trust");
        Ok(())
    }

    fn migrated_db() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn tracked_memory(files: Option<&str>) -> Memory {
        let mut memory = memory(1_700_000_000, "active");
        memory.session_id = Some("mem-session-1".to_string());
        memory.project = "proj".to_string();
        memory.files = files.map(str::to_string);
        memory
    }

    fn link_commit(
        conn: &Connection,
        id: i64,
        sha: &str,
        epoch: i64,
        changed_files: &[&str],
        memory_session_id: &str,
    ) -> Result<()> {
        insert_commit(conn, id, sha, epoch, changed_files)?;
        conn.execute(
            "INSERT INTO git_commit_sessions
             (commit_id, session_id, memory_session_id, source, linked_at_epoch)
             VALUES (?1, ?2, ?3, 'test', ?4)",
            params![id, format!("content-{id}"), memory_session_id, epoch],
        )?;
        Ok(())
    }

    fn insert_commit(
        conn: &Connection,
        id: i64,
        sha: &str,
        epoch: i64,
        changed_files: &[&str],
    ) -> Result<()> {
        let changed_files = serde_json::to_string(changed_files)?;
        conn.execute(
            "INSERT INTO git_commits
             (id, project, repo_path, sha, short_sha, branch, message,
              authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
             VALUES (?1, 'proj', '/repo', ?2, ?2, 'main', NULL, ?3, ?4, ?3, ?3)",
            params![id, sha, epoch, changed_files],
        )?;
        Ok(())
    }
}
