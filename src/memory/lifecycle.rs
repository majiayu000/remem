use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLifecycleOp {
    Add,
    Update,
    Invalidate,
    Noop,
    Defer,
}

impl MemoryLifecycleOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Invalidate => "invalidate",
            Self::Noop => "noop",
            Self::Defer => "defer",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleOutcome {
    pub op: MemoryLifecycleOp,
    pub memory_id: Option<i64>,
    pub superseded: usize,
    pub noop: bool,
    pub deferred: bool,
    pub reason: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn apply_add(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
) -> Result<LifecycleOutcome> {
    let memory_id = crate::memory::insert_memory_full(
        conn,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        branch,
        scope,
        None,
    )?;
    Ok(LifecycleOutcome {
        op: MemoryLifecycleOp::Add,
        memory_id: Some(memory_id),
        superseded: 0,
        noop: false,
        deferred: false,
        reason: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn apply_update(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    topic_key: &str,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    superseded_ids: &[i64],
) -> Result<LifecycleOutcome> {
    let memory_id = crate::memory::insert_memory_full(
        conn,
        session_id,
        project,
        Some(topic_key),
        title,
        content,
        memory_type,
        files,
        branch,
        scope,
        None,
    )?;
    let superseded = soft_supersede(conn, project, superseded_ids, Some(memory_id))?;
    Ok(LifecycleOutcome {
        op: MemoryLifecycleOp::Update,
        memory_id: Some(memory_id),
        superseded,
        noop: false,
        deferred: false,
        reason: None,
    })
}

pub fn apply_invalidate(
    conn: &Connection,
    project: &str,
    memory_ids: &[i64],
    reason: Option<&str>,
) -> Result<LifecycleOutcome> {
    let superseded = soft_supersede(conn, project, memory_ids, None)?;
    Ok(LifecycleOutcome {
        op: MemoryLifecycleOp::Invalidate,
        memory_id: None,
        superseded,
        noop: false,
        deferred: false,
        reason: reason.map(str::to_string),
    })
}

pub fn noop(reason: impl Into<String>) -> LifecycleOutcome {
    LifecycleOutcome {
        op: MemoryLifecycleOp::Noop,
        memory_id: None,
        superseded: 0,
        noop: true,
        deferred: false,
        reason: Some(reason.into()),
    }
}

pub fn defer(reason: impl Into<String>) -> LifecycleOutcome {
    LifecycleOutcome {
        op: MemoryLifecycleOp::Defer,
        memory_id: None,
        superseded: 0,
        noop: false,
        deferred: true,
        reason: Some(reason.into()),
    }
}

pub fn soft_supersede(
    conn: &Connection,
    project: &str,
    memory_ids: &[i64],
    replacement_id: Option<i64>,
) -> Result<usize> {
    let mut seen = std::collections::HashSet::with_capacity(memory_ids.len());
    let mut changed = 0usize;
    for id in memory_ids
        .iter()
        .copied()
        .filter(|id| Some(*id) != replacement_id && seen.insert(*id))
    {
        let updated = conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1 AND project = ?2",
            params![id, project],
        )?;
        if updated != 1 {
            return Err(anyhow!(
                "failed to mark superseded memory stale: id={} project={}",
                id,
                project
            ));
        }
        changed += updated;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::memory::insert_memory;
    use crate::memory::tests_helper::setup_memory_schema;
    use crate::retrieval::search::search_with_branch;

    #[test]
    fn update_preserves_superseded_memory_but_default_search_returns_current_fact() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-lifecycle";
        let old_id = insert_memory(
            &conn,
            Some("s1"),
            project,
            Some("deploy-target"),
            "Deploy target",
            "Deploy target is staging.",
            "decision",
            None,
        )?;

        let outcome = apply_update(
            &conn,
            Some("s2"),
            project,
            "deploy-target-current",
            "Deploy target corrected",
            "Deploy target is production.",
            "decision",
            None,
            None,
            "project",
            &[old_id],
        )?;

        assert_eq!(outcome.op, MemoryLifecycleOp::Update);
        assert_eq!(outcome.superseded, 1);
        let old_status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            [old_id],
            |row| row.get(0),
        )?;
        assert_eq!(old_status, "stale");

        let results = search_with_branch(
            &conn,
            Some("deploy target"),
            Some(project),
            None,
            10,
            0,
            false,
            None,
        )?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "Deploy target is production.");
        Ok(())
    }

    #[test]
    fn invalidate_marks_memory_stale_without_deleting_it() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-lifecycle";
        let id = insert_memory(
            &conn,
            Some("s1"),
            project,
            Some("old-fact"),
            "Old fact",
            "This fact is no longer valid.",
            "discovery",
            None,
        )?;

        let outcome = apply_invalidate(&conn, project, &[id], Some("contradicted"))?;
        assert_eq!(outcome.op, MemoryLifecycleOp::Invalidate);
        assert_eq!(outcome.superseded, 1);

        let (status, content): (String, String) = conn.query_row(
            "SELECT status, content FROM memories WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(status, "stale");
        assert_eq!(content, "This fact is no longer valid.");
        Ok(())
    }

    #[test]
    fn noop_and_defer_are_explicit_outcomes() {
        assert!(noop("duplicate").noop);
        assert!(defer("ambiguous conflict").deferred);
    }
}
