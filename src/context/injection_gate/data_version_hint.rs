use anyhow::Result;
use rusqlite::types::ToSql;
use sha2::{Digest, Sha256};
use std::path::Path;

use super::super::invocation::ContextInvocation;
use super::super::policy::{ContextPolicy, SectionKind};
use super::super::types::ContextRequest;

const SUMMARY_VERSION_SCAN_LIMIT: i64 = 200;
const BASENAME_SEARCH_LIMIT: i64 = 20;
const COMMIT_VERSION_SCAN_LIMIT: i64 = 200;
const FACT_VERSION_SCAN_LIMIT: i64 = 500;
const DATA_VERSION_HINT_SCHEMA: &str = "v3";

pub(in crate::context) fn compute_data_version_hint(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    invocation: &ContextInvocation,
    policy: &ContextPolicy,
) -> Result<String> {
    let mut version = DataVersionHintBuilder::new();
    version.push("version", DATA_VERSION_HINT_SCHEMA);
    version.push("project", &request.project);
    version.push("cwd", &request.cwd);
    version.push("branch", request.current_branch.as_deref().unwrap_or(""));
    version.push("session", request.session_id.as_deref().unwrap_or(""));
    version.push("source", request.hook_source.as_deref().unwrap_or(""));
    version.push("host", request.host.as_env_value());
    version.push("colors", if request.use_colors { "1" } else { "0" });
    version.push("gate_mode", invocation.gate_mode.as_deref().unwrap_or(""));
    version.push("package", env!("CARGO_PKG_VERSION"));
    version.push(
        "schema",
        &crate::migrate::latest_schema_version().to_string(),
    );
    version.push("policy", &format!("{:?}", policy));
    version.push("claude_md", &claude_md_fingerprint(&request.cwd)?);

    let now = chrono::Utc::now().timestamp();
    version.push("day_bucket", &(now / 86_400).to_string());
    version.push(
        "hybrid_substrate",
        &super::hybrid_substrate_hint::compute_hybrid_substrate_fingerprint(
            conn,
            &request.project,
        )?,
    );
    push_commit_signal(conn, request, &mut version)?;
    push_memory_signal(conn, request, now, policy, &mut version)?;
    push_memory_state_key_signal(conn, &request.project, &mut version)?;
    push_memory_fact_signal(conn, &request.project, &mut version)?;
    push_lesson_signal(conn, request, policy, now, &mut version)?;
    push_preference_signal(conn, &request.project, policy, &mut version)?;
    push_session_signal(conn, &request.project, &mut version)?;
    push_workstream_signal(conn, &request.project, policy, &mut version)?;

    Ok(version.finish())
}

fn claude_md_fingerprint(cwd: &str) -> Result<String> {
    let path = Path::new(cwd).join("CLAUDE.md");
    match std::fs::read(&path) {
        Ok(bytes) => Ok(super::sha256_hex_bytes(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("missing".to_string()),
        Err(error) => Err(error.into()),
    }
}

fn push_commit_signal(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let recent_messages = super::super::commit_signals::query_recent_commit_messages(
        conn,
        &request.project,
        request.current_branch.as_deref(),
        3,
    )?;
    for message in &recent_messages {
        version.push("recent_commit_message", message);
    }
    version.push(
        "recent_commit_messages_count",
        &recent_messages.len().to_string(),
    );

    if !crate::retrieval::temporal::sqlite_table_exists(conn, "git_commits")? {
        version.push("git_commits_table", "missing");
        return Ok(());
    }
    version.push("git_commits_table", "present");
    let summary = conn.query_row(
        "SELECT COUNT(*), MAX(id), MAX(updated_at_epoch)
         FROM git_commits
         WHERE project = ?1",
        [&request.project],
        |row| {
            Ok(vec![
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, Option<i64>>(1)?
                    .unwrap_or_default()
                    .to_string(),
                row.get::<_, Option<i64>>(2)?
                    .unwrap_or_default()
                    .to_string(),
            ])
        },
    )?;
    version.push_row("git_commits_summary", &summary);
    let mut stmt = conn.prepare(
        "SELECT id, sha, short_sha, branch, message, authored_at_epoch,
                changed_files, created_at_epoch, updated_at_epoch
         FROM git_commits
         WHERE project = ?1
         ORDER BY COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![request.project, COMMIT_VERSION_SCAN_LIMIT],
        |row| {
            Ok(vec![
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                row.get::<_, Option<i64>>(5)?
                    .unwrap_or_default()
                    .to_string(),
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?.to_string(),
                row.get::<_, i64>(8)?.to_string(),
            ])
        },
    )?;
    push_rows("git_commit", rows, version)?;

    if !crate::retrieval::temporal::sqlite_table_exists(conn, "git_commit_sessions")? {
        version.push("git_commit_sessions_table", "missing");
        return Ok(());
    }
    version.push("git_commit_sessions_table", "present");
    let summary = conn.query_row(
        "SELECT COUNT(*), MAX(l.commit_id), MAX(l.linked_at_epoch)
         FROM git_commit_sessions l
         JOIN git_commits c ON c.id = l.commit_id
         WHERE c.project = ?1",
        [&request.project],
        |row| {
            Ok(vec![
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, Option<i64>>(1)?
                    .unwrap_or_default()
                    .to_string(),
                row.get::<_, Option<i64>>(2)?
                    .unwrap_or_default()
                    .to_string(),
            ])
        },
    )?;
    version.push_row("git_commit_sessions_summary", &summary);
    let mut stmt = conn.prepare(
        "SELECT l.commit_id, l.session_id, l.memory_session_id, l.source, l.linked_at_epoch
         FROM git_commit_sessions l
         JOIN git_commits c ON c.id = l.commit_id
         WHERE c.project = ?1
         ORDER BY l.linked_at_epoch DESC, l.commit_id DESC, l.session_id
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![request.project, COMMIT_VERSION_SCAN_LIMIT],
        |row| {
            Ok(vec![
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?.to_string(),
            ])
        },
    )?;
    push_rows("git_commit_session", rows, version)
}

fn push_memory_signal(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    now: i64,
    policy: &ContextPolicy,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let excluded_types = policy
        .section(SectionKind::MemoryIndex)
        .map(|section| section.exclude_types.as_slice())
        .unwrap_or(&[]);
    let recent_type_filter = memory_type_filter(excluded_types, 4);
    let recent_sql = memory_window_sql(&recent_type_filter, None, 4 + excluded_types.len());
    let recent_params = memory_window_params(
        request,
        now,
        None,
        excluded_types,
        policy.limits.candidate_fetch_limit as i64,
    );
    push_memory_window(
        conn,
        "memories_recent",
        &recent_sql,
        &recent_params,
        version,
    )?;

    let basename = request
        .project
        .rsplit('/')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&request.project);
    let like_pattern = format!("%{basename}%");
    let search_type_filter = memory_type_filter(excluded_types, 5);
    let search_sql = memory_window_sql(
        &search_type_filter,
        Some(" AND (m.title LIKE ?4 OR m.content LIKE ?4)"),
        5 + excluded_types.len(),
    );
    let search_params = memory_window_params(
        request,
        now,
        Some(like_pattern),
        excluded_types,
        BASENAME_SEARCH_LIMIT,
    );
    push_memory_window(
        conn,
        "memories_basename",
        &search_sql,
        &search_params,
        version,
    )
}

fn push_memory_state_key_signal(
    conn: &rusqlite::Connection,
    project: &str,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, owner_scope, owner_key, memory_type, state_key, state_label,
                state_status, current_memory_id, created_at_epoch, updated_at_epoch
         FROM memory_state_keys
         WHERE (owner_scope = 'repo' AND owner_key = ?1)
            OR (owner_scope = 'user' AND owner_key = 'user:default')
         ORDER BY owner_scope, owner_key, memory_type, state_key, id",
    )?;
    let rows = stmt.query_map([project], |row| {
        Ok(vec![
            row.get::<_, i64>(0)?.to_string(),
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            row.get::<_, String>(6)?,
            row.get::<_, Option<i64>>(7)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, i64>(8)?.to_string(),
            row.get::<_, i64>(9)?.to_string(),
        ])
    })?;
    push_rows("memory_state_keys", rows, version)?;
    Ok(())
}

fn push_memory_fact_signal(
    conn: &rusqlite::Connection,
    project: &str,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    if !crate::retrieval::temporal::sqlite_table_exists(conn, "memory_facts")? {
        version.push("memory_facts_table", "missing");
        return Ok(());
    }
    version.push("memory_facts_table", "present");
    let has_invalidated_at_epoch = crate::memory::facts::invalidated_at_epoch_available(conn)?;
    version.push(
        "memory_facts_invalidated_at_epoch",
        if has_invalidated_at_epoch { "1" } else { "0" },
    );
    let summary = conn.query_row(
        "SELECT COUNT(*), MAX(id), MAX(updated_at_epoch)
         FROM memory_facts
         WHERE project = ?1",
        [project],
        |row| {
            Ok(vec![
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, Option<i64>>(1)?
                    .unwrap_or_default()
                    .to_string(),
                row.get::<_, Option<i64>>(2)?
                    .unwrap_or_default()
                    .to_string(),
            ])
        },
    )?;
    version.push_row("memory_facts_summary", &summary);
    let invalidated_expr = if has_invalidated_at_epoch {
        "invalidated_at_epoch"
    } else {
        "NULL"
    };
    let sql = format!(
        "SELECT id, project, subject, predicate, object, valid_from_epoch,
                valid_to_epoch, learned_at_epoch, source_memory_id, source_observation_id,
                source_event_ids, printf('%.6f', confidence), supersedes_fact_id, status,
                {invalidated_expr}, created_at_epoch, updated_at_epoch
         FROM memory_facts
         WHERE project = ?1
         ORDER BY updated_at_epoch DESC, id DESC
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![project, FACT_VERSION_SCAN_LIMIT], |row| {
        Ok(vec![
            row.get::<_, i64>(0)?.to_string(),
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<i64>>(5)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, Option<i64>>(6)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, i64>(7)?.to_string(),
            row.get::<_, Option<i64>>(8)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, Option<i64>>(9)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, String>(10)?,
            row.get::<_, String>(11)?,
            row.get::<_, Option<i64>>(12)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, String>(13)?,
            row.get::<_, Option<i64>>(14)?
                .unwrap_or_default()
                .to_string(),
            row.get::<_, i64>(15)?.to_string(),
            row.get::<_, i64>(16)?.to_string(),
        ])
    })?;
    push_rows("memory_fact", rows, version)
}

fn push_lesson_signal(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    policy: &ContextPolicy,
    now: i64,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let lessons = crate::memory::lesson::list_lessons_for_context(
        conn,
        &request.project,
        request.current_branch.as_deref(),
        policy.section_item_limit(SectionKind::Lessons, policy.limits.lesson_limit) as i64,
    )?;
    for lesson in &lessons {
        push_memory(version, "lesson", &lesson.memory);
        version.push_row(
            "lesson_meta",
            &[
                lesson.metadata.memory_id.to_string(),
                format!("{:.6}", lesson.metadata.confidence),
                lesson.metadata.reinforcement_count.to_string(),
                lesson.metadata.source_evidence.clone().unwrap_or_default(),
                lesson.metadata.last_reinforced_at_epoch.to_string(),
                lesson
                    .metadata
                    .stale_after_epoch
                    .unwrap_or_default()
                    .to_string(),
            ],
        );
    }
    version.push("lessons_count", &lessons.len().to_string());
    version.push("lesson_now_bucket", &(now / 86_400).to_string());
    version.push("lesson_limit", &policy.limits.lesson_limit.to_string());
    Ok(())
}

fn push_preference_signal(
    conn: &rusqlite::Connection,
    project: &str,
    policy: &ContextPolicy,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let project_preferences = crate::memory::preference::query_project_preferences(
        conn,
        project,
        policy.limits.preference_project_limit,
    )?;
    for memory in &project_preferences {
        push_memory(version, "preference_project", memory);
    }
    version.push(
        "preferences_project_count",
        &project_preferences.len().to_string(),
    );

    let global_preferences = crate::memory::preference::query_global_preferences(
        conn,
        policy.limits.preference_global_limit,
    )?;
    for memory in &global_preferences {
        push_memory(version, "preference_global", memory);
    }
    version.push(
        "preferences_global_count",
        &global_preferences.len().to_string(),
    );
    Ok(())
}

fn push_session_signal(
    conn: &rusqlite::Connection,
    project: &str,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id, created_at_epoch, request, completed
         FROM session_summaries
         WHERE request IS NOT NULL AND request != ''
           AND session_row_id IS NULL
           AND ((owner_scope = 'repo' AND owner_key = ?1)
                OR (owner_scope = 'repo' AND target_project = ?1)
                OR (owner_scope IS NULL AND project = ?1))
         ORDER BY created_at_epoch DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![project, SUMMARY_VERSION_SCAN_LIMIT],
        |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?.unwrap_or_default(),
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        },
    )?;

    let mut count = 0_i64;
    for row in rows {
        let (id, created_at_epoch, request, completed) = row?;
        count += 1;
        version.push(
            "session",
            &format!("{id}|{created_at_epoch}|{request}|{completed}"),
        );
    }
    version.push("sessions_count", &count.to_string());
    Ok(())
}

fn push_workstream_signal(
    conn: &rusqlite::Connection,
    project: &str,
    policy: &ContextPolicy,
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let limit = policy.section_item_limit(SectionKind::Workstreams, 5);
    if limit == 0 {
        version.push("workstreams_count", "0");
        return Ok(());
    }

    let mut stmt = conn.prepare(
        "SELECT id, project, title, description, status, progress, next_action,
                blockers, created_at_epoch, updated_at_epoch, completed_at_epoch
         FROM workstreams
         WHERE status = 'active'
           AND merged_into_workstream_id IS NULL
           AND ((owner_scope = 'repo' AND owner_key = ?1)
                OR (owner_scope = 'repo' AND target_project = ?1)
                OR (owner_scope = 'workstream' AND target_project = ?1)
                OR (owner_scope IS NULL AND project = ?1))
         ORDER BY updated_at_epoch DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![project, limit as i64], |row| {
        Ok(vec![
            row.get::<_, i64>(0)?.to_string(),
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            row.get::<_, Option<String>>(6)?.unwrap_or_default(),
            row.get::<_, Option<String>>(7)?.unwrap_or_default(),
            row.get::<_, i64>(8)?.to_string(),
            row.get::<_, i64>(9)?.to_string(),
            row.get::<_, Option<i64>>(10)?
                .unwrap_or_default()
                .to_string(),
        ])
    })?;

    let mut count = 0usize;
    for row in rows {
        version.push_row("workstream", &row?);
        count += 1;
    }
    version.push("workstreams_count", &count.to_string());
    Ok(())
}

fn push_rows<I>(label: &'static str, rows: I, version: &mut DataVersionHintBuilder) -> Result<()>
where
    I: Iterator<Item = rusqlite::Result<Vec<String>>>,
{
    let mut count = 0usize;
    for row in rows {
        version.push_row(label, &row?);
        count += 1;
    }
    version.push(&format!("{label}_count"), &count.to_string());
    Ok(())
}

fn memory_type_filter(excluded_types: &[&str], first_param_index: usize) -> String {
    if excluded_types.is_empty() {
        return String::new();
    }

    let placeholders = (first_param_index..first_param_index + excluded_types.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(" AND m.memory_type NOT IN ({placeholders})")
}

fn memory_window_params(
    request: &ContextRequest,
    now: i64,
    search_pattern: Option<String>,
    excluded_types: &[&str],
    limit: i64,
) -> Vec<Box<dyn ToSql>> {
    let mut params: Vec<Box<dyn ToSql>> = vec![
        Box::new(request.project.clone()),
        Box::new(now),
        Box::new(request.current_branch.clone()),
    ];
    if let Some(pattern) = search_pattern {
        params.push(Box::new(pattern));
    }
    for excluded_type in excluded_types {
        params.push(Box::new((*excluded_type).to_string()));
    }
    params.push(Box::new(limit));
    params
}

fn memory_window_sql(
    type_filter: &str,
    search_filter: Option<&str>,
    limit_param_index: usize,
) -> String {
    format!(
        "SELECT m.id, m.session_id, m.project, m.topic_key, m.title, m.content,
                m.memory_type, m.files, m.created_at_epoch, m.updated_at_epoch,
                m.status, m.branch, m.scope, m.source_project, m.target_project,
                m.owner_scope, m.owner_key, m.context_class, m.expires_at_epoch,
                m.valid_from_epoch, m.valid_to_epoch, m.state_key_id, m.search_context
         FROM memories m
         WHERE m.status = 'active'
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?2)
           AND (m.state_key_id IS NULL OR NOT EXISTS (
                SELECT 1 FROM memory_state_keys sk
                WHERE sk.id = m.state_key_id
                  AND sk.current_memory_id IS NOT NULL
                  AND sk.current_memory_id <> m.id
           ))
           AND (?3 IS NULL OR m.branch = ?3 OR m.branch IS NULL)
           AND ((m.owner_scope = 'repo' AND m.owner_key = ?1)
                OR (m.owner_scope = 'repo' AND m.target_project = ?1)
                OR (m.owner_scope IS NULL AND m.project = ?1
                    AND COALESCE(m.scope, 'project') != 'global'))
           {type_filter}
           {}
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?{}",
        search_filter.unwrap_or(""),
        limit_param_index
    )
}

fn push_memory_window(
    conn: &rusqlite::Connection,
    label: &'static str,
    sql: &str,
    params: &[Box<dyn ToSql>],
    version: &mut DataVersionHintBuilder,
) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(
        rusqlite::params_from_iter(crate::db::to_sql_refs(params)),
        map_memory_version_row,
    )?;
    let mut count = 0usize;
    for row in rows {
        version.push_row(label, &row?);
        count += 1;
    }
    version.push(&format!("{label}_count"), &count.to_string());
    Ok(())
}

fn map_memory_version_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Vec<String>> {
    Ok(vec![
        row.get::<_, i64>(0)?.to_string(),
        row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        row.get::<_, String>(2)?,
        row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        row.get::<_, String>(4)?,
        row.get::<_, String>(5)?,
        row.get::<_, String>(6)?,
        row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        row.get::<_, i64>(8)?.to_string(),
        row.get::<_, i64>(9)?.to_string(),
        row.get::<_, String>(10)?,
        row.get::<_, Option<String>>(11)?.unwrap_or_default(),
        row.get::<_, String>(12)?,
        row.get::<_, Option<String>>(13)?.unwrap_or_default(),
        row.get::<_, Option<String>>(14)?.unwrap_or_default(),
        row.get::<_, Option<String>>(15)?.unwrap_or_default(),
        row.get::<_, Option<String>>(16)?.unwrap_or_default(),
        row.get::<_, Option<String>>(17)?.unwrap_or_default(),
        row.get::<_, Option<i64>>(18)?
            .unwrap_or_default()
            .to_string(),
        row.get::<_, Option<i64>>(19)?
            .unwrap_or_default()
            .to_string(),
        row.get::<_, Option<i64>>(20)?
            .unwrap_or_default()
            .to_string(),
        row.get::<_, Option<i64>>(21)?
            .unwrap_or_default()
            .to_string(),
        row.get::<_, Option<String>>(22)?.unwrap_or_default(),
    ])
}

fn push_memory(
    version: &mut DataVersionHintBuilder,
    label: &'static str,
    memory: &crate::memory::Memory,
) {
    version.push_row(
        label,
        &[
            memory.id.to_string(),
            memory.session_id.clone().unwrap_or_default(),
            memory.project.clone(),
            memory.topic_key.clone().unwrap_or_default(),
            memory.title.clone(),
            memory.text.clone(),
            memory.memory_type.clone(),
            memory.files.clone().unwrap_or_default(),
            memory.created_at_epoch.to_string(),
            memory.updated_at_epoch.to_string(),
            memory.status.clone(),
            memory.branch.clone().unwrap_or_default(),
            memory.scope.clone(),
        ],
    );
}

struct DataVersionHintBuilder {
    hasher: Sha256,
}

impl DataVersionHintBuilder {
    fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }

    fn push(&mut self, key: &str, value: &str) {
        self.hasher.update(key.as_bytes());
        self.hasher.update([0]);
        self.hasher.update(value.as_bytes());
        self.hasher.update([0xff]);
    }

    fn push_row(&mut self, label: &str, fields: &[String]) {
        self.push(label, &fields.len().to_string());
        for field in fields {
            self.push("field", field);
        }
        self.push("row_end", label);
    }

    fn finish(self) -> String {
        format!("{DATA_VERSION_HINT_SCHEMA}:{:x}", self.hasher.finalize())
    }
}
