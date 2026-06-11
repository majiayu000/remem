use anyhow::Result;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::path::Path;

use super::super::invocation::ContextInvocation;
use super::super::policy::{ContextPolicy, SectionKind};
use super::super::types::ContextRequest;

const SUMMARY_VERSION_SCAN_LIMIT: i64 = 200;
const BASENAME_SEARCH_LIMIT: i64 = 20;
const INDEXED_MEMORY_TYPES_SQL: &str = "'decision', 'discovery', 'bugfix', 'architecture', \
                                        'procedure', 'session_activity'";

pub(super) fn context_injections_has_data_version(conn: &rusqlite::Connection) -> Result<bool> {
    conn.query_row(
        "SELECT 1
         FROM pragma_table_info('context_injections')
         WHERE name = 'data_version'
         LIMIT 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|value| value.is_some())
    .map_err(Into::into)
}

pub(super) fn compute_data_version(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    invocation: &ContextInvocation,
    policy: &ContextPolicy,
) -> Result<String> {
    let mut version = DataVersionBuilder::new();
    version.push("version", "2");
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
    push_memory_signal(conn, request, now, policy, &mut version)?;
    push_memory_state_key_signal(conn, &request.project, &mut version)?;
    push_lesson_signal(conn, request, policy, now, &mut version)?;
    push_preference_signal(conn, &request.project, policy, &mut version)?;
    push_session_signal(conn, &request.project, &mut version)?;
    push_workstream_signal(conn, &request.project, &mut version)?;

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

fn push_memory_signal(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    now: i64,
    policy: &ContextPolicy,
    version: &mut DataVersionBuilder,
) -> Result<()> {
    let excluded_types = policy
        .section(SectionKind::MemoryIndex)
        .map(|section| section.exclude_types.as_slice())
        .unwrap_or(&[]);
    let type_filter = if excluded_types.is_empty() {
        String::new()
    } else {
        format!(" AND m.memory_type IN ({})", INDEXED_MEMORY_TYPES_SQL)
    };
    let recent_sql = memory_window_sql(&type_filter, None);
    push_memory_window(
        conn,
        "memories_recent",
        &recent_sql,
        rusqlite::params![
            request.project,
            now,
            request.current_branch.as_deref(),
            policy.limits.candidate_fetch_limit as i64
        ],
        version,
    )?;

    let basename = request
        .project
        .rsplit('/')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&request.project);
    let like_pattern = format!("%{basename}%");
    let search_sql = memory_window_sql(
        &type_filter,
        Some(" AND (m.title LIKE ?4 OR m.content LIKE ?4)"),
    );
    push_memory_window(
        conn,
        "memories_basename",
        &search_sql,
        rusqlite::params![
            request.project,
            now,
            request.current_branch.as_deref(),
            like_pattern,
            BASENAME_SEARCH_LIMIT
        ],
        version,
    )
}

fn push_memory_state_key_signal(
    conn: &rusqlite::Connection,
    project: &str,
    version: &mut DataVersionBuilder,
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

fn push_lesson_signal(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    policy: &ContextPolicy,
    now: i64,
    version: &mut DataVersionBuilder,
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
    version: &mut DataVersionBuilder,
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
    version: &mut DataVersionBuilder,
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
    version: &mut DataVersionBuilder,
) -> Result<()> {
    let workstreams = crate::workstream::query_active_workstreams(conn, project)?;
    for workstream in &workstreams {
        version.push_row(
            "workstream",
            &[
                workstream.id.to_string(),
                workstream.project.clone(),
                workstream.title.clone(),
                workstream.description.clone().unwrap_or_default(),
                workstream.status.as_str().to_string(),
                workstream.progress.clone().unwrap_or_default(),
                workstream.next_action.clone().unwrap_or_default(),
                workstream.blockers.clone().unwrap_or_default(),
                workstream.created_at_epoch.to_string(),
                workstream.updated_at_epoch.to_string(),
                workstream
                    .completed_at_epoch
                    .unwrap_or_default()
                    .to_string(),
            ],
        );
    }
    version.push("workstreams_count", &workstreams.len().to_string());
    Ok(())
}

fn memory_window_sql(type_filter: &str, search_filter: Option<&str>) -> String {
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
        if search_filter.is_some() { 5 } else { 4 }
    )
}

fn push_memory_window<P>(
    conn: &rusqlite::Connection,
    label: &'static str,
    sql: &str,
    params: P,
    version: &mut DataVersionBuilder,
) -> Result<()>
where
    P: rusqlite::Params,
{
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, map_memory_version_row)?;
    let mut count = 0usize;
    for row in rows {
        push_memory_fields(version, label, &row?);
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
    version: &mut DataVersionBuilder,
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

fn push_memory_fields(version: &mut DataVersionBuilder, label: &'static str, fields: &[String]) {
    version.push_row(label, fields);
}

fn push_rows<I>(label: &'static str, rows: I, version: &mut DataVersionBuilder) -> Result<()>
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

struct DataVersionBuilder {
    hasher: Sha256,
}

impl DataVersionBuilder {
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
        format!("{:x}", self.hasher.finalize())
    }
}
