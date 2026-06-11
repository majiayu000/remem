use anyhow::Result;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::path::Path;

use super::super::invocation::ContextInvocation;
use super::super::policy::ContextPolicy;
use super::super::types::ContextRequest;

const SUMMARY_VERSION_SCAN_LIMIT: i64 = 200;

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
    push_memory_signal(conn, request, now, &mut version)?;
    push_memory_state_key_signal(conn, &request.project, &mut version)?;
    push_lesson_signal(conn, request, policy, now, &mut version)?;
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
    version: &mut DataVersionBuilder,
) -> Result<()> {
    let signal: AggregateSignal = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(MAX(m.updated_at_epoch), 0),
                COALESCE(SUM(m.id), 0)
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
                OR (m.owner_scope = 'user' AND m.owner_key = 'user:default')
                OR (m.owner_scope IS NULL AND (
                    (m.project = ?1 AND COALESCE(m.scope, 'project') != 'global')
                    OR m.scope = 'global'
                )))",
        rusqlite::params![request.project, now, request.current_branch.as_deref()],
        map_aggregate_signal,
    )?;
    signal.push("memories", version);
    Ok(())
}

fn push_memory_state_key_signal(
    conn: &rusqlite::Connection,
    project: &str,
    version: &mut DataVersionBuilder,
) -> Result<()> {
    let signal: AggregateSignal = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(MAX(updated_at_epoch), 0),
                COALESCE(SUM(COALESCE(current_memory_id, 0)), 0)
         FROM memory_state_keys
         WHERE (owner_scope = 'repo' AND owner_key = ?1)
            OR (owner_scope = 'user' AND owner_key = 'user:default')",
        [project],
        map_aggregate_signal,
    )?;
    signal.push("memory_state_keys", version);
    Ok(())
}

fn push_lesson_signal(
    conn: &rusqlite::Connection,
    request: &ContextRequest,
    policy: &ContextPolicy,
    now: i64,
    version: &mut DataVersionBuilder,
) -> Result<()> {
    let signal: LessonSignal = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(MAX(m.updated_at_epoch), 0),
                COALESCE(SUM(m.id), 0),
                COALESCE(MAX(l.last_reinforced_at_epoch), 0),
                COALESCE(SUM(l.reinforcement_count), 0),
                COALESCE(SUM(CAST(l.confidence * 1000000 AS INTEGER)), 0),
                COALESCE(MAX(COALESCE(l.stale_after_epoch, 0)), 0)
         FROM memories m
         JOIN memory_lessons l ON l.memory_id = m.id
         WHERE m.memory_type = 'lesson'
           AND m.status = 'active'
           AND (m.expires_at_epoch IS NULL OR m.expires_at_epoch > ?2)
           AND (m.state_key_id IS NULL OR NOT EXISTS (
                SELECT 1 FROM memory_state_keys sk
                WHERE sk.id = m.state_key_id
                  AND sk.current_memory_id IS NOT NULL
                  AND sk.current_memory_id <> m.id
           ))
           AND ((m.owner_scope = 'repo' AND m.owner_key = ?1)
                OR (m.owner_scope = 'repo' AND m.target_project = ?1)
                OR (m.owner_scope = 'user' AND m.owner_key = 'user:default')
                OR (m.owner_scope IS NULL AND (m.project = ?1 OR m.scope = 'global')))
           AND l.confidence >= 0.5
           AND (l.stale_after_epoch IS NULL OR l.stale_after_epoch > ?2)
           AND (?3 IS NULL OR m.branch = ?3 OR m.branch IS NULL)",
        rusqlite::params![request.project, now, request.current_branch.as_deref()],
        |row| {
            Ok(LessonSignal {
                count: row.get(0)?,
                max_memory_updated: row.get(1)?,
                id_sum: row.get(2)?,
                max_reinforced: row.get(3)?,
                reinforcement_sum: row.get(4)?,
                confidence_sum: row.get(5)?,
                max_stale_after: row.get(6)?,
            })
        },
    )?;
    signal.push(version);
    version.push("lesson_limit", &policy.limits.lesson_limit.to_string());
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
    let signal: AggregateSignal = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(MAX(updated_at_epoch), 0),
                COALESCE(SUM(id), 0)
         FROM workstreams
         WHERE status = 'active'
           AND ((owner_scope = 'repo' AND owner_key = ?1)
                OR (owner_scope = 'repo' AND target_project = ?1)
                OR (owner_scope = 'workstream' AND target_project = ?1)
                OR (owner_scope IS NULL AND project = ?1))",
        [project],
        map_aggregate_signal,
    )?;
    signal.push("workstreams", version);
    Ok(())
}

fn map_aggregate_signal(row: &rusqlite::Row<'_>) -> rusqlite::Result<AggregateSignal> {
    Ok(AggregateSignal {
        count: row.get(0)?,
        max_epoch: row.get(1)?,
        id_sum: row.get(2)?,
    })
}

struct AggregateSignal {
    count: i64,
    max_epoch: i64,
    id_sum: i64,
}

impl AggregateSignal {
    fn push(&self, label: &'static str, version: &mut DataVersionBuilder) {
        version.push(
            label,
            &format!("{}|{}|{}", self.count, self.max_epoch, self.id_sum),
        );
    }
}

struct LessonSignal {
    count: i64,
    max_memory_updated: i64,
    id_sum: i64,
    max_reinforced: i64,
    reinforcement_sum: i64,
    confidence_sum: i64,
    max_stale_after: i64,
}

impl LessonSignal {
    fn push(&self, version: &mut DataVersionBuilder) {
        version.push(
            "lessons",
            &format!(
                "{}|{}|{}|{}|{}|{}|{}",
                self.count,
                self.max_memory_updated,
                self.id_sum,
                self.max_reinforced,
                self.reinforcement_sum,
                self.confidence_sum,
                self.max_stale_after
            ),
        );
    }
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

    fn finish(self) -> String {
        format!("{:x}", self.hasher.finalize())
    }
}
