use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use super::promote::slugify_for_topic;
use super::types::{map_memory_row_pub, Memory, MEMORY_COLS};

const MIN_CONFIDENCE_FOR_CONTEXT: f64 = 0.5;

#[derive(Debug, Clone)]
pub struct LessonMetadata {
    pub memory_id: i64,
    pub confidence: f64,
    pub reinforcement_count: i64,
    pub source_evidence: Option<String>,
    pub last_reinforced_at_epoch: i64,
    pub stale_after_epoch: Option<i64>,
    pub outcome_kind: String,
    pub success_count: i64,
    pub failure_count: i64,
    pub recovery_count: i64,
    pub correction_count: i64,
    pub revert_count: i64,
}

#[derive(Debug, Clone)]
pub struct LessonMemory {
    pub memory: Memory,
    pub metadata: LessonMetadata,
}

#[derive(Debug, Clone)]
pub struct SaveLessonRequest<'a> {
    pub session_id: Option<&'a str>,
    pub project: &'a str,
    pub topic_key: Option<&'a str>,
    pub title: &'a str,
    pub content: &'a str,
    pub confidence: f64,
    pub source_evidence: Option<&'a str>,
    pub files: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub scope: &'a str,
    pub created_at_epoch: Option<i64>,
    pub stale_after_epoch: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LessonOutcomeKind {
    Unknown,
    Success,
    Failure,
    Recovery,
    Correction,
    Revert,
}

impl LessonOutcomeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Recovery => "recovery",
            Self::Correction => "correction",
            Self::Revert => "revert",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LessonOutcomeUpdate {
    kind: LessonOutcomeKind,
    success_delta: i64,
    failure_delta: i64,
    recovery_delta: i64,
    correction_delta: i64,
    revert_delta: i64,
}

impl LessonOutcomeUpdate {
    pub fn unknown() -> Self {
        Self {
            kind: LessonOutcomeKind::Unknown,
            success_delta: 0,
            failure_delta: 0,
            recovery_delta: 0,
            correction_delta: 0,
            revert_delta: 0,
        }
    }

    pub fn success() -> Self {
        Self {
            kind: LessonOutcomeKind::Success,
            success_delta: 1,
            ..Self::unknown()
        }
    }

    pub fn failure() -> Self {
        Self {
            kind: LessonOutcomeKind::Failure,
            failure_delta: 1,
            ..Self::unknown()
        }
    }

    pub fn recovery() -> Self {
        Self {
            kind: LessonOutcomeKind::Recovery,
            recovery_delta: 1,
            ..Self::unknown()
        }
    }

    pub fn correction() -> Self {
        Self {
            kind: LessonOutcomeKind::Correction,
            correction_delta: 1,
            ..Self::unknown()
        }
    }

    pub fn revert() -> Self {
        Self {
            kind: LessonOutcomeKind::Revert,
            revert_delta: 1,
            ..Self::unknown()
        }
    }
}

pub fn save_lesson(conn: &Connection, req: &SaveLessonRequest<'_>) -> Result<i64> {
    save_lesson_with_reference_time(conn, req, req.created_at_epoch)
}

pub fn save_lesson_with_outcome(
    conn: &Connection,
    req: &SaveLessonRequest<'_>,
    outcome: LessonOutcomeUpdate,
) -> Result<i64> {
    save_lesson_with_reference_time_and_outcome(conn, req, req.created_at_epoch, outcome)
}

pub fn save_lesson_with_reference_time(
    conn: &Connection,
    req: &SaveLessonRequest<'_>,
    reference_time_epoch: Option<i64>,
) -> Result<i64> {
    save_lesson_with_reference_time_and_outcome(
        conn,
        req,
        reference_time_epoch,
        LessonOutcomeUpdate::unknown(),
    )
}

fn save_lesson_with_reference_time_and_outcome(
    conn: &Connection,
    req: &SaveLessonRequest<'_>,
    reference_time_epoch: Option<i64>,
    outcome: LessonOutcomeUpdate,
) -> Result<i64> {
    validate_outcome_update(outcome)?;
    let topic_key = req
        .topic_key
        .map(str::to_string)
        .unwrap_or_else(|| format!("lesson-{}", slugify_for_topic(req.content, 64)));
    let scope = if req.scope.trim().is_empty() {
        "project"
    } else {
        req.scope
    };
    let existing_id = existing_lesson_id(conn, req.project, &topic_key, scope)?;
    let id = crate::memory::insert_memory_full_with_reference_time(
        conn,
        req.session_id,
        req.project,
        Some(&topic_key),
        req.title,
        req.content,
        "lesson",
        req.files,
        req.branch,
        scope,
        req.created_at_epoch,
        reference_time_epoch,
    )?;
    let metadata_exists = get_lesson_metadata(conn, id)?.is_some();
    upsert_lesson_metadata(
        conn,
        id,
        req,
        existing_id.is_some() || metadata_exists,
        outcome,
    )?;
    Ok(id)
}

fn validate_outcome_update(outcome: LessonOutcomeUpdate) -> Result<()> {
    for (name, value) in [
        ("success_delta", outcome.success_delta),
        ("failure_delta", outcome.failure_delta),
        ("recovery_delta", outcome.recovery_delta),
        ("correction_delta", outcome.correction_delta),
        ("revert_delta", outcome.revert_delta),
    ] {
        if value < 0 {
            anyhow::bail!("lesson outcome {name} must be non-negative");
        }
    }
    Ok(())
}

fn existing_lesson_id(
    conn: &Connection,
    project: &str,
    topic_key: &str,
    scope: &str,
) -> Result<Option<i64>> {
    let id = conn
        .query_row(
            "SELECT id FROM memories
             WHERE project = ?1
               AND topic_key = ?2
               AND COALESCE(scope, 'project') = ?3
               AND memory_type = 'lesson'
             LIMIT 1",
            params![project, topic_key, scope],
            |row| row.get(0),
        )
        .optional()?;
    Ok(id)
}

fn upsert_lesson_metadata(
    conn: &Connection,
    memory_id: i64,
    req: &SaveLessonRequest<'_>,
    existed: bool,
    outcome: LessonOutcomeUpdate,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let confidence = req.confidence.clamp(0.0, 1.0);
    let reinforcement_delta = if existed { 1 } else { 0 };
    conn.execute(
        "INSERT INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch, outcome_kind,
          success_count, failure_count, recovery_count, correction_count, revert_count)
         VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(memory_id) DO UPDATE SET
           confidence = MAX(memory_lessons.confidence, excluded.confidence),
           reinforcement_count = memory_lessons.reinforcement_count + ?12,
           source_evidence = COALESCE(excluded.source_evidence, memory_lessons.source_evidence),
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           stale_after_epoch = excluded.stale_after_epoch,
           outcome_kind = CASE
             WHEN excluded.outcome_kind != 'unknown' THEN excluded.outcome_kind
             ELSE memory_lessons.outcome_kind
           END,
           success_count = memory_lessons.success_count + excluded.success_count,
           failure_count = memory_lessons.failure_count + excluded.failure_count,
           recovery_count = memory_lessons.recovery_count + excluded.recovery_count,
           correction_count = memory_lessons.correction_count + excluded.correction_count,
           revert_count = memory_lessons.revert_count + excluded.revert_count",
        params![
            memory_id,
            confidence,
            req.source_evidence,
            now,
            req.stale_after_epoch,
            outcome.kind.as_str(),
            outcome.success_delta,
            outcome.failure_delta,
            outcome.recovery_delta,
            outcome.correction_delta,
            outcome.revert_delta,
            reinforcement_delta
        ],
    )?;
    Ok(())
}

pub fn get_lesson_metadata(conn: &Connection, memory_id: i64) -> Result<Option<LessonMetadata>> {
    conn.query_row(
        "SELECT memory_id, confidence, reinforcement_count, source_evidence,
                last_reinforced_at_epoch, stale_after_epoch, outcome_kind,
                success_count, failure_count, recovery_count, correction_count, revert_count
         FROM memory_lessons WHERE memory_id = ?1",
        [memory_id],
        map_lesson_metadata_row,
    )
    .optional()
    .map_err(Into::into)
}

pub fn list_lessons_for_context(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    limit: i64,
) -> Result<Vec<LessonMemory>> {
    if limit <= 0 {
        return Ok(vec![]);
    }
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare(&format!(
        "SELECT {cols},
                l.memory_id, l.confidence, l.reinforcement_count, l.source_evidence,
                l.last_reinforced_at_epoch, l.stale_after_epoch, l.outcome_kind,
                l.success_count, l.failure_count, l.recovery_count, l.correction_count,
                l.revert_count
         FROM memories m
         JOIN memory_lessons l ON l.memory_id = m.id
         WHERE m.memory_type = 'lesson'
           AND {current_filter}
           AND {policy_filter}
           AND ((m.owner_scope = 'repo' AND m.owner_key = ?1)
                OR (m.owner_scope = 'repo' AND m.target_project = ?1)
                OR (m.owner_scope = 'user' AND m.owner_key = 'user:default')
                OR (m.owner_scope IS NULL AND (m.project = ?1 OR m.scope = 'global')))
           AND l.confidence >= ?2
           AND (l.stale_after_epoch IS NULL OR l.stale_after_epoch > ?3)
           AND (?4 IS NULL OR m.branch = ?4 OR m.branch IS NULL)
         ORDER BY
           CASE WHEN m.project = ?1 THEN 0 ELSE 1 END,
           l.confidence DESC,
           l.reinforcement_count DESC,
           l.last_reinforced_at_epoch DESC,
           m.id ASC
         LIMIT ?5",
        cols = prefixed_memory_cols("m"),
        current_filter =
            crate::memory::memory_current_filter_sql("m.status", "m.expires_at_epoch", false),
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("m"),
    ))?;
    let rows = stmt.query_map(
        params![
            project,
            MIN_CONFIDENCE_FOR_CONTEXT,
            now,
            current_branch,
            limit
        ],
        |row| {
            let memory = map_memory_row_pub(row)?;
            let metadata = map_lesson_metadata_from_offset(row, 13)?;
            Ok(LessonMemory { memory, metadata })
        },
    )?;
    crate::db::query::collect_rows(rows)
}

pub fn is_lesson_candidate(text: &str) -> bool {
    let normalized = text.trim().to_lowercase();
    if normalized.len() < 30 {
        return false;
    }
    const SIGNALS: &[&str] = &[
        "lesson:",
        "root cause",
        "avoid ",
        "do not ",
        "don't ",
        "never ",
        "must ",
        "should ",
        "proven ",
        "workflow",
        "pitfall",
        "prevent",
        "regression",
    ];
    SIGNALS.iter().any(|signal| normalized.contains(signal))
}

fn prefixed_memory_cols(alias: &str) -> String {
    MEMORY_COLS
        .split(',')
        .map(|col| format!("{alias}.{}", col.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn map_lesson_metadata_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LessonMetadata> {
    map_lesson_metadata_from_offset(row, 0)
}

fn map_lesson_metadata_from_offset(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<LessonMetadata> {
    Ok(LessonMetadata {
        memory_id: row.get(offset)?,
        confidence: row.get(offset + 1)?,
        reinforcement_count: row.get(offset + 2)?,
        source_evidence: row.get(offset + 3)?,
        last_reinforced_at_epoch: row.get(offset + 4)?,
        stale_after_epoch: row.get(offset + 5)?,
        outcome_kind: row.get(offset + 6)?,
        success_count: row.get(offset + 7)?,
        failure_count: row.get(offset + 8)?,
        recovery_count: row.get(offset + 9)?,
        correction_count: row.get(offset + 10)?,
        revert_count: row.get(offset + 11)?,
    })
}

#[cfg(test)]
mod tests;
