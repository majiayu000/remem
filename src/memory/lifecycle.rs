use anyhow::Result;
use rusqlite::{params, Connection};

use crate::memory::state_key::StateKeyDecision;

mod supersede;
pub use supersede::soft_supersede;
use supersede::soft_supersede_owned;

pub const SHORT_CURRENT_TTL_SECONDS: i64 = 24 * 60 * 60;
pub const BRANCH_SNAPSHOT_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

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
    let tx = conn.unchecked_transaction()?;
    let ownership = lifecycle_ownership(project, scope);
    let state_key =
        crate::memory::state_key::derive_state_key(memory_type, Some(topic_key), title, content);
    let mut superseded_targets = superseded_ids.to_vec();
    superseded_targets.extend(find_active_same_state_or_topic(
        &tx,
        &ownership,
        memory_type,
        topic_key,
        state_key.as_ref(),
    )?);
    let memory_id = insert_replacement_memory(
        &tx,
        session_id,
        project,
        topic_key,
        title,
        content,
        memory_type,
        files,
        branch,
        scope,
        &ownership,
        state_key.as_ref(),
    )?;
    let superseded = soft_supersede_owned(&tx, &ownership, &superseded_targets, Some(memory_id))?;
    tx.commit()?;
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
    let tx = conn.unchecked_transaction()?;
    let superseded = soft_supersede(&tx, project, memory_ids, None)?;
    tx.commit()?;
    Ok(LifecycleOutcome {
        op: MemoryLifecycleOp::Invalidate,
        memory_id: None,
        superseded,
        noop: false,
        deferred: false,
        reason: reason.map(str::to_string),
    })
}

#[allow(clippy::too_many_arguments)]
fn insert_replacement_memory(
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
    ownership: &LifecycleOwnership<'_>,
    state_key: Option<&StateKeyDecision>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let (expires_at_epoch, valid_from_epoch) =
        ttl_metadata(memory_type, Some(topic_key), content, now);
    let search_context = crate::memory::search_context::build_search_context(
        memory_type,
        Some(topic_key),
        content,
        files,
    );
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, status, branch, scope,
          source_project, target_project, owner_scope, owner_key, context_class,
          expires_at_epoch, valid_from_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                 ?9, ?9, 'active', ?10, ?11,
                 ?12, ?13, ?14, ?15, 'startup_core',
                 ?16, ?17)",
        params![
            session_id,
            project,
            topic_key,
            title,
            content,
            memory_type,
            files,
            search_context,
            now,
            branch,
            scope,
            ownership.source_project,
            ownership.target_project,
            ownership.owner_scope,
            ownership.owner_key,
            expires_at_epoch,
            valid_from_epoch
        ],
    )?;
    let memory_id = conn.last_insert_rowid();
    if let Some(state_key) = state_key {
        crate::memory::state_key::attach_current_memory(
            conn,
            memory_id,
            ownership.owner_scope,
            ownership.owner_key,
            memory_type,
            state_key,
            now,
        )?;
    }
    Ok(memory_id)
}

struct LifecycleOwnership<'a> {
    source_project: &'a str,
    target_project: Option<&'a str>,
    owner_scope: &'static str,
    owner_key: &'a str,
}

fn lifecycle_ownership<'a>(project: &'a str, scope: &str) -> LifecycleOwnership<'a> {
    if scope == "global" {
        LifecycleOwnership {
            source_project: project,
            target_project: None,
            owner_scope: "user",
            owner_key: "user:default",
        }
    } else {
        LifecycleOwnership {
            source_project: project,
            target_project: Some(project),
            owner_scope: "repo",
            owner_key: project,
        }
    }
}

fn find_active_same_state_or_topic(
    conn: &Connection,
    ownership: &LifecycleOwnership<'_>,
    memory_type: &str,
    topic_key: &str,
    state_key: Option<&StateKeyDecision>,
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    if let Some(state_key) = state_key {
        ids.extend(crate::memory::state_key::active_memory_ids(
            conn,
            ownership.owner_scope,
            ownership.owner_key,
            memory_type,
            &state_key.state_key,
            chrono::Utc::now().timestamp(),
            false,
        )?);
    }
    ids.extend(find_active_same_topic_key(
        conn,
        ownership,
        memory_type,
        topic_key,
    )?);
    Ok(ids)
}

fn find_active_same_topic_key(
    conn: &Connection,
    ownership: &LifecycleOwnership<'_>,
    memory_type: &str,
    topic_key: &str,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM memories
         WHERE memory_type = ?1
           AND topic_key = ?2
           AND COALESCE(
                owner_scope,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
           ) = ?3
           AND COALESCE(
                owner_key,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END
           ) = ?4
           AND status = 'active'",
    )?;
    let rows = stmt.query_map(
        params![
            memory_type,
            topic_key,
            ownership.owner_scope,
            ownership.owner_key
        ],
        |row| row.get(0),
    )?;
    crate::db::query::collect_rows(rows)
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

pub fn default_ttl_seconds(
    memory_type: &str,
    topic_key: Option<&str>,
    content: &str,
) -> Option<i64> {
    let topic_key = topic_key.unwrap_or_default().to_ascii_lowercase();
    let content = content.to_ascii_lowercase();

    if has_any(&topic_key, short_current_needles()) {
        return Some(SHORT_CURRENT_TTL_SECONDS);
    }

    if has_any(&topic_key, branch_snapshot_needles()) {
        return Some(BRANCH_SNAPSHOT_TTL_SECONDS);
    }

    if durable_type_has_no_content_ttl(memory_type) {
        return None;
    }

    if has_any(&content, short_current_needles()) {
        return Some(SHORT_CURRENT_TTL_SECONDS);
    }

    if has_any(&content, branch_snapshot_needles()) {
        return Some(BRANCH_SNAPSHOT_TTL_SECONDS);
    }

    None
}

pub fn expires_at_epoch(
    memory_type: &str,
    topic_key: Option<&str>,
    content: &str,
    now_epoch: i64,
) -> Option<i64> {
    default_ttl_seconds(memory_type, topic_key, content).map(|ttl| now_epoch + ttl)
}

pub fn ttl_metadata(
    memory_type: &str,
    topic_key: Option<&str>,
    content: &str,
    now_epoch: i64,
) -> (Option<i64>, Option<i64>) {
    let expires_at_epoch = expires_at_epoch(memory_type, topic_key, content, now_epoch);
    let valid_from_epoch = expires_at_epoch.map(|_| now_epoch);
    (expires_at_epoch, valid_from_epoch)
}

pub fn expire_active_memories(conn: &Connection, now_epoch: i64) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE memories
         SET status = 'stale',
             valid_to_epoch = COALESCE(valid_to_epoch, ?1),
             updated_at_epoch = ?1
         WHERE status = 'active'
           AND expires_at_epoch IS NOT NULL
           AND expires_at_epoch <= ?1",
        params![now_epoch],
    )?)
}

fn has_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn short_current_needles() -> &'static [&'static str] {
    &[
        "dev-server",
        "dev server",
        "localhost",
        "127.0.0.1",
        "port occupied",
        "port is occupied",
        "currently running",
        "server running",
        "local url",
        "url healthy",
        "healthy at",
        "mergeability",
        "mergeable",
        "review status",
        "review-status",
        "ci state",
        "ci status",
        "ci-status",
        "github actions",
        "pull request",
        "pull-request",
        "pr #",
    ]
}

fn branch_snapshot_needles() -> &'static [&'static str] {
    &[
        "git-divergence",
        "branch-divergence",
        "branch divergence",
        "current branch",
        "git status",
        "ahead of",
        "behind origin",
        "diverged",
        "dirty worktree",
    ]
}

fn durable_type_has_no_content_ttl(memory_type: &str) -> bool {
    matches!(
        memory_type,
        "architecture" | "bugfix" | "lesson" | "preference" | "procedure"
    )
}

#[cfg(test)]
mod ttl_tests;

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
    fn global_state_key_update_supersedes_previous_repo_owner_row() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let repo_a = "/tmp/repo-a";
        let repo_b = "/tmp/repo-b";
        let old_id = crate::memory::insert_memory_full(
            &conn,
            Some("s1"),
            repo_a,
            Some("preference-aaaaaaaa"),
            "Preference",
            "Keep verification status separate from data and code changes.",
            "preference",
            None,
            None,
            "global",
            None,
        )?;

        let outcome = apply_update(
            &conn,
            Some("s2"),
            repo_b,
            "preference-bbbbbbbb",
            "Preference",
            "Report data and code changes separately from verification status.",
            "preference",
            None,
            None,
            "global",
            &[],
        )?;
        let new_id = outcome
            .memory_id
            .ok_or_else(|| anyhow::anyhow!("replacement id missing"))?;

        assert_ne!(old_id, new_id);
        assert_eq!(outcome.superseded, 1);
        let old_status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            [old_id],
            |row| row.get(0),
        )?;
        assert_eq!(old_status, "stale");
        let (project, status, owner_scope, owner_key, state_key, current_memory_id): (
            String,
            String,
            String,
            String,
            String,
            i64,
        ) = conn.query_row(
            "SELECT m.project, m.status, m.owner_scope, m.owner_key, sk.state_key,
                    sk.current_memory_id
             FROM memories m
             JOIN memory_state_keys sk ON sk.id = m.state_key_id
             WHERE m.id = ?1",
            [new_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        assert_eq!(project, repo_b);
        assert_eq!(status, "active");
        assert_eq!(owner_scope, "user");
        assert_eq!(owner_key, "user:default");
        assert_eq!(state_key, "verification-status-separation");
        assert_eq!(current_memory_id, new_id);
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
    fn update_rolls_back_insert_when_superseded_id_is_invalid() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-lifecycle";
        let old_id = insert_memory(
            &conn,
            Some("s1"),
            project,
            Some("old-fact"),
            "Old fact",
            "Old value.",
            "decision",
            None,
        )?;

        let err = apply_update(
            &conn,
            Some("s2"),
            project,
            "new-fact",
            "New fact",
            "New value.",
            "decision",
            None,
            None,
            "project",
            &[old_id, 999_999],
        )
        .expect_err("invalid superseded id should fail");
        assert!(err.to_string().contains("999999") || err.to_string().contains("999_999"));

        let active_new_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = 'new-fact'",
            [project],
            |row| row.get(0),
        )?;
        let old_status: String = conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            [old_id],
            |row| row.get(0),
        )?;
        assert_eq!(active_new_count, 0);
        assert_eq!(old_status, "active");
        Ok(())
    }

    #[test]
    fn invalidate_rolls_back_when_any_memory_id_is_invalid() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_memory_schema(&conn);
        let project = "test-lifecycle";
        let first_id = insert_memory(
            &conn,
            Some("s1"),
            project,
            Some("first"),
            "First",
            "First value.",
            "discovery",
            None,
        )?;
        let second_id = insert_memory(
            &conn,
            Some("s1"),
            project,
            Some("second"),
            "Second",
            "Second value.",
            "discovery",
            None,
        )?;

        apply_invalidate(
            &conn,
            project,
            &[first_id, 999_999, second_id],
            Some("bad id"),
        )
        .expect_err("mixed-validity invalidation should fail");

        let statuses = conn
            .prepare("SELECT status FROM memories WHERE id IN (?1, ?2) ORDER BY id ASC")?
            .query_map([first_id, second_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(statuses, vec!["active".to_string(), "active".to_string()]);
        Ok(())
    }

    #[test]
    fn noop_and_defer_are_explicit_outcomes() {
        assert!(noop("duplicate").noop);
        assert!(defer("ambiguous conflict").deferred);
    }
}
