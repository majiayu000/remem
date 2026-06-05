use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::memory::lifecycle::MemoryLifecycleOp;

pub const PLANNER_VERSION: &str = "memory-operation-planner-v1";

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryOperationInput {
    pub source: String,
    pub actor: String,
    pub source_project: String,
    pub owner_scope: String,
    pub owner_key: String,
    pub memory_type: String,
    pub topic_key: Option<String>,
    pub state_key: Option<String>,
    pub source_candidate_id: Option<i64>,
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryOperationPlan {
    pub op: MemoryLifecycleOp,
    pub state_key: Option<String>,
    pub target_memory_id: Option<i64>,
    pub superseded_ids: Vec<i64>,
    pub conflicting_ids: Vec<i64>,
    pub noop_reason: Option<String>,
    pub defer_reason: Option<String>,
    pub planner_version: &'static str,
    pub reason: String,
}

impl MemoryOperationPlan {
    pub fn new(
        op: MemoryLifecycleOp,
        state_key: Option<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            op,
            state_key,
            target_memory_id: None,
            superseded_ids: Vec::new(),
            conflicting_ids: Vec::new(),
            noop_reason: None,
            defer_reason: None,
            planner_version: PLANNER_VERSION,
            reason: reason.into(),
        }
    }

    pub fn with_target_memory_id(mut self, memory_id: Option<i64>) -> Self {
        self.target_memory_id = memory_id;
        self
    }

    pub fn with_superseded_ids(mut self, superseded_ids: Vec<i64>) -> Self {
        self.superseded_ids = superseded_ids;
        self
    }

    pub fn with_noop_reason(mut self, reason: impl Into<String>) -> Self {
        self.noop_reason = Some(reason.into());
        self
    }

    pub fn with_defer_reason(mut self, reason: impl Into<String>) -> Self {
        self.defer_reason = Some(reason.into());
        self
    }
}

pub fn owner_for_scope(project: &str, scope: &str) -> (&'static str, String) {
    if scope == "global" {
        ("user", "user:default".to_string())
    } else {
        ("repo", project.to_string())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn plan_direct_save(
    conn: &Connection,
    source: &str,
    actor: &str,
    project: &str,
    scope: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    files: Option<&str>,
    branch: Option<&str>,
    source_candidate_id: Option<i64>,
    confidence: Option<f64>,
) -> Result<(MemoryOperationInput, MemoryOperationPlan)> {
    let now = chrono::Utc::now().timestamp();
    let (owner_scope, owner_key) = owner_for_scope(project, scope);
    let state_key =
        crate::memory::state_key::derive_state_key(memory_type, topic_key, title, content)
            .map(|decision| decision.state_key);
    let existing = existing_memory_for_direct_save(
        conn,
        project,
        scope,
        memory_type,
        topic_key,
        state_key.as_deref(),
        now,
    )?;
    let input = MemoryOperationInput {
        source: source.to_string(),
        actor: actor.to_string(),
        source_project: project.to_string(),
        owner_scope: owner_scope.to_string(),
        owner_key,
        memory_type: memory_type.to_string(),
        topic_key: topic_key.map(str::to_string),
        state_key: state_key.clone(),
        source_candidate_id,
        confidence,
    };
    let plan = match existing {
        Some(existing) if existing.matches_noop_write(title, content, files, branch, now) => {
            MemoryOperationPlan::new(
                MemoryLifecycleOp::Noop,
                state_key,
                "existing active memory already represents this fact",
            )
            .with_target_memory_id(Some(existing.id))
            .with_noop_reason("already represented by active memory")
        }
        Some(existing) => MemoryOperationPlan::new(
            MemoryLifecycleOp::Update,
            state_key,
            "existing state/topic memory will be updated",
        )
        .with_target_memory_id(Some(existing.id)),
        None => MemoryOperationPlan::new(
            MemoryLifecycleOp::Add,
            state_key,
            "no existing state/topic memory; add new durable memory",
        ),
    };
    Ok((input, plan))
}

pub fn operation_for_memory_write(conn: &Connection, memory_id: i64) -> Result<MemoryLifecycleOp> {
    let created_updated: (i64, i64) = conn
        .query_row(
            "SELECT created_at_epoch, updated_at_epoch FROM memories WHERE id = ?1",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .with_context(|| format!("load memory timestamps for operation result id={memory_id}"))?;
    Ok(if created_updated.0 == created_updated.1 {
        MemoryLifecycleOp::Add
    } else {
        MemoryLifecycleOp::Update
    })
}

pub fn insert_operation_log(
    conn: &Connection,
    input: &MemoryOperationInput,
    plan: &MemoryOperationPlan,
    result_memory_id: Option<i64>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let superseded_ids = serde_json::to_string(&plan.superseded_ids)
        .context("serialize memory operation superseded ids")?;
    let conflicting_ids = serde_json::to_string(&plan.conflicting_ids)
        .context("serialize memory operation conflicting ids")?;
    conn.execute(
        "INSERT INTO memory_operation_log
         (operation, planner_version, actor, source, owner_scope, owner_key,
          memory_type, state_key, input_topic_key, source_candidate_id, result_memory_id,
          superseded_ids, conflicting_ids, noop_reason, defer_reason, confidence, reason,
          created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6,
                 ?7, ?8, ?9, ?10, ?11,
                 ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18)",
        params![
            plan.op.as_str(),
            plan.planner_version,
            input.actor.as_str(),
            input.source.as_str(),
            input.owner_scope.as_str(),
            input.owner_key.as_str(),
            input.memory_type.as_str(),
            plan.state_key.as_deref().or(input.state_key.as_deref()),
            input.topic_key.as_deref(),
            input.source_candidate_id,
            result_memory_id.or(plan.target_memory_id),
            superseded_ids,
            conflicting_ids,
            plan.noop_reason.as_deref(),
            plan.defer_reason.as_deref(),
            input.confidence,
            plan.reason.as_str(),
            now
        ],
    )
    .context("insert memory operation audit log")?;
    Ok(conn.last_insert_rowid())
}

pub fn with_operation_savepoint<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT remem_memory_operation")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_memory_operation")?;
            Ok(value)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_memory_operation;
                 RELEASE SAVEPOINT remem_memory_operation;",
            );
            if let Err(rollback_error) = rollback {
                return Err(error.context(format!(
                    "memory operation rollback also failed: {rollback_error}"
                )));
            }
            Err(error)
        }
    }
}

#[derive(Debug)]
struct ExistingMemory {
    id: i64,
    title: String,
    content: String,
    status: String,
    files: Option<String>,
    branch: Option<String>,
    expires_at_epoch: Option<i64>,
}

impl ExistingMemory {
    fn matches_noop_write(
        &self,
        title: &str,
        content: &str,
        files: Option<&str>,
        branch: Option<&str>,
        now_epoch: i64,
    ) -> bool {
        self.status == "active"
            && self.is_current(now_epoch)
            && self.title == title
            && same_memory_text(&self.content, content)
            && self.files.as_deref() == files
            && self.branch.as_deref() == branch
    }

    fn is_current(&self, now_epoch: i64) -> bool {
        match self.expires_at_epoch {
            Some(expires_at_epoch) => expires_at_epoch > now_epoch,
            None => true,
        }
    }
}

fn existing_memory_for_direct_save(
    conn: &Connection,
    project: &str,
    scope: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    state_key: Option<&str>,
    now_epoch: i64,
) -> Result<Option<ExistingMemory>> {
    if let Some(topic_key) = topic_key.filter(|topic_key| !topic_key.is_empty()) {
        let existing_id = conn
            .query_row(
                "SELECT id, title, content, status, files, branch, expires_at_epoch FROM memories
                 WHERE project = ?1 AND topic_key = ?2 AND scope = ?3
                 ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END,
                          updated_at_epoch DESC,
                          id DESC
                 LIMIT 1",
                params![project, topic_key, scope],
                map_existing_memory,
            )
            .optional()
            .context("check existing durable memory for topic_key upsert")?;
        if existing_id.is_some() {
            return Ok(existing_id);
        }
    }

    let Some(state_key) = state_key else {
        return Ok(None);
    };
    let (owner_scope, owner_key) = owner_for_scope(project, scope);
    let Some(id) = crate::memory::state_key::current_memory_id(
        conn,
        owner_scope,
        &owner_key,
        memory_type,
        state_key,
        now_epoch,
    )?
    else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT id, title, content, status, files, branch, expires_at_epoch
         FROM memories WHERE id = ?1",
        [id],
        map_existing_memory,
    )
    .optional()
    .with_context(|| format!("load existing memory for state key id={id}"))
}

fn map_existing_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExistingMemory> {
    Ok(ExistingMemory {
        id: row.get(0)?,
        title: row.get(1)?,
        content: row.get(2)?,
        status: row.get(3)?,
        files: row.get(4)?,
        branch: row.get(5)?,
        expires_at_epoch: row.get(6)?,
    })
}

pub fn same_memory_text(left: &str, right: &str) -> bool {
    normalize_memory_text(left) == normalize_memory_text(right)
}

fn normalize_memory_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}
