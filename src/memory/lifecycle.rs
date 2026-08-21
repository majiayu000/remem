use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};

use crate::memory::state_key::StateKeyDecision;

pub const SHORT_CURRENT_TTL_SECONDS: i64 = 24 * 60 * 60;
pub const BRANCH_SNAPSHOT_TTL_SECONDS: i64 = 7 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryLifecycleOp {
    Add,
    Update,
    Invalidate,
    Noop,
    Defer,
    Conflict,
}

impl MemoryLifecycleOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Invalidate => "invalidate",
            Self::Noop => "noop",
            Self::Defer => "defer",
            Self::Conflict => "conflict",
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
        project,
        branch,
        scope,
        &ownership,
        memory_type,
        topic_key,
        state_key.as_ref(),
    )?);
    superseded_targets.sort_unstable();
    superseded_targets.dedup();
    if let Some(matched) =
        crate::memory::poisoning::scan_instruction_pattern(&format!("{title}\n{content}"))
    {
        anyhow::bail!(
            "lifecycle update payload matched instruction-pattern {}@{}",
            matched.pattern_id,
            matched.pattern_set_version
        );
    }
    let superseded_json = serde_json::to_string(&superseded_targets)?;
    let payload_sha256 = crate::memory::activation::payload_sha256(&[
        project,
        topic_key,
        title,
        content,
        memory_type,
        files.unwrap_or(""),
        branch.unwrap_or(""),
        scope,
        &superseded_json,
    ]);
    let request = crate::memory::activation::ActiveMemoryWriteRequest {
        activation_id: crate::memory::activation::ephemeral_activation_id(
            "lifecycle-update",
            &payload_sha256,
        ),
        route_kind: crate::memory::activation::ActivationRouteKind::RustApi,
        actor_kind: crate::memory::activation::ActivationActorKind::RustApi,
        source_operation: "memory_lifecycle_update".to_string(),
        source_trust: crate::memory::poisoning::SourceTrustClass::LocalToolOutput,
        source_project: ownership.source_project.to_string(),
        route: crate::memory::activation::ActiveMemoryRoute {
            project: project.to_string(),
            branch: branch.map(str::to_string),
            scope: scope.to_string(),
            owner_scope: ownership.owner_scope.to_string(),
            owner_key: ownership.owner_key.to_string(),
            target_project: ownership.target_project.map(str::to_string),
        },
        provenance_kind: crate::memory::activation::ActivationProvenanceKind::RustApi,
        provenance_ref: "rust-api:lifecycle-update:v1".to_string(),
        payload_sha256,
        expected_memory: crate::memory::activation::ExpectedActiveMemory::new(
            title,
            content,
            memory_type,
        )
        .with_topic_key(Some(topic_key))
        .with_files(files),
        poisoning_verdict: crate::memory::activation::ActivationPoisoningVerdict::Clean,
        superseded_ids: superseded_targets.clone(),
    };
    let mut superseded = None;
    let activation_result = crate::memory::activation::execute_one(&tx, &request, |permit| {
        let memory_id = insert_replacement_memory(
            &tx,
            permit,
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
        let count = soft_supersede(&tx, project, &superseded_targets, Some(memory_id))?;
        crate::memory::edge::insert_supersedes_edges(
            &tx,
            &superseded_targets,
            memory_id,
            crate::memory::edge::MemoryEdgeWriteContext {
                reason: Some("lifecycle update supersedes old memory"),
                ..Default::default()
            },
        )?;
        superseded = Some(count);
        Ok(memory_id)
    })?;
    tx.commit()?;
    Ok(LifecycleOutcome {
        op: MemoryLifecycleOp::Update,
        memory_id: Some(activation_result.memory_id),
        superseded: superseded.unwrap_or(0),
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
    _permit: &crate::memory::activation::ActiveMemoryWritePermit,
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
    let fallback_source_hash = crate::memory::retrieval_enrichment::enrichment_source_hash(
        title,
        content,
        memory_type,
        Some(topic_key),
        files,
    );
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          search_context_fallback_source_hash,
          created_at_epoch, updated_at_epoch, status, branch, scope,
          source_project, target_project, owner_scope, owner_key, context_class,
          expires_at_epoch, valid_from_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?18,
                 ?9, ?9, 'active', ?10, ?11,
                 ?12, ?13, ?14, ?15, 'startup_core', ?16, ?17)",
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
            valid_from_epoch,
            fallback_source_hash
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
    crate::retrieval::vector::upsert_memory_embedding(
        conn,
        memory_id,
        title,
        content,
        memory_type,
        Some(topic_key),
        "",
    )?;
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
    project: &str,
    branch: Option<&str>,
    scope: &str,
    ownership: &LifecycleOwnership<'_>,
    memory_type: &str,
    topic_key: &str,
    state_key: Option<&StateKeyDecision>,
) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    if let Some(state_key) = state_key {
        let candidates = crate::memory::state_key::active_memory_ids(
            conn,
            ownership.owner_scope,
            ownership.owner_key,
            memory_type,
            &state_key.state_key,
            chrono::Utc::now().timestamp(),
            false,
        )?;
        for memory_id in candidates {
            if matches_lifecycle_route(conn, memory_id, project, branch, scope, ownership)? {
                ids.push(memory_id);
            }
        }
    }
    ids.extend(find_active_same_topic_key(
        conn,
        project,
        branch,
        scope,
        ownership,
        memory_type,
        topic_key,
    )?);
    Ok(ids)
}

fn find_active_same_topic_key(
    conn: &Connection,
    project: &str,
    branch: Option<&str>,
    scope: &str,
    ownership: &LifecycleOwnership<'_>,
    memory_type: &str,
    topic_key: &str,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM memories
         WHERE memory_type = ?1 AND topic_key = ?2
           AND project = ?3 AND branch IS ?4 AND COALESCE(scope, 'project') = ?5
           AND COALESCE(owner_scope,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END) = ?6
           AND COALESCE(owner_key,
                CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END) = ?7
           AND target_project IS ?8
           AND status = 'active'",
    )?;
    let rows = stmt.query_map(
        params![
            memory_type,
            topic_key,
            project,
            branch,
            scope,
            ownership.owner_scope,
            ownership.owner_key,
            ownership.target_project,
        ],
        |row| row.get(0),
    )?;
    crate::db::query::collect_rows(rows)
}

fn matches_lifecycle_route(
    conn: &Connection,
    memory_id: i64,
    project: &str,
    branch: Option<&str>,
    scope: &str,
    ownership: &LifecycleOwnership<'_>,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM memories
             WHERE id = ?1 AND status = 'active' AND project = ?2
               AND branch IS ?3 AND COALESCE(scope, 'project') = ?4
               AND COALESCE(owner_scope,
                   CASE WHEN COALESCE(scope, 'project') = 'global'
                        THEN 'user' ELSE 'repo' END) = ?5
               AND COALESCE(owner_key,
                   CASE WHEN COALESCE(scope, 'project') = 'global'
                        THEN 'user:default' ELSE project END) = ?6
               AND target_project IS ?7
         )",
        params![
            memory_id,
            project,
            branch,
            scope,
            ownership.owner_scope,
            ownership.owner_key,
            ownership.target_project,
        ],
        |row| row.get(0),
    )
    .map_err(Into::into)
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
    if !conn.is_autocommit() {
        return expire_active_memories_in_transaction(conn, now_epoch);
    }
    let tx = rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)?;
    let changed = expire_active_memories_in_transaction(&tx, now_epoch)?;
    tx.commit()?;
    Ok(changed)
}

fn expire_active_memories_in_transaction(conn: &Connection, now_epoch: i64) -> Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id FROM memories
         WHERE status = 'active'
           AND memory_type = 'preference'
           AND expires_at_epoch IS NOT NULL
           AND expires_at_epoch <= ?1",
    )?;
    let rows = stmt.query_map(params![now_epoch], |row| row.get::<_, i64>(0))?;
    let expiring_preference_ids = crate::db::query::collect_rows(rows)?;
    drop(stmt);
    let changed = conn.execute(
        "UPDATE memories
         SET status = 'stale',
             valid_to_epoch = COALESCE(valid_to_epoch, ?1),
             updated_at_epoch = ?1
         WHERE status = 'active'
           AND expires_at_epoch IS NOT NULL
           AND expires_at_epoch <= ?1",
        params![now_epoch],
    )?;
    crate::memory::preference::compilation::enqueue_for_memory_ids(conn, &expiring_preference_ids)?;
    Ok(changed)
}

pub fn count_expired_active_memories(conn: &Connection, now_epoch: i64) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE status = 'active'
           AND expires_at_epoch IS NOT NULL
           AND expires_at_epoch <= ?1",
        params![now_epoch],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

pub fn soft_supersede(
    conn: &Connection,
    project: &str,
    memory_ids: &[i64],
    replacement_id: Option<i64>,
) -> Result<usize> {
    let mut seen = std::collections::HashSet::with_capacity(memory_ids.len());
    let targets = memory_ids
        .iter()
        .copied()
        .filter(|id| Some(*id) != replacement_id && seen.insert(*id))
        .collect::<Vec<_>>();
    for id in &targets {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM memories WHERE id = ?1 AND project = ?2)",
            params![id, project],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(anyhow!(
                "failed to mark superseded memory stale: id={} project={}",
                id,
                project
            ));
        }
    }

    crate::memory::preference::compilation::enqueue_for_memory_ids(conn, &targets)?;

    let mut changed = 0usize;
    let now = chrono::Utc::now().timestamp();
    for id in targets {
        let updated = conn.execute(
            "UPDATE memories
             SET status = 'stale',
                 valid_to_epoch = COALESCE(valid_to_epoch, ?3)
             WHERE id = ?1 AND project = ?2",
            params![id, project, now],
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
mod vector_tests;
