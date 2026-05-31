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

pub fn save_lesson(conn: &Connection, req: &SaveLessonRequest<'_>) -> Result<i64> {
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
    let id = crate::memory::insert_memory_full(
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
    )?;
    upsert_lesson_metadata(conn, id, req, existing_id.is_some())?;
    Ok(id)
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
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let confidence = req.confidence.clamp(0.0, 1.0);
    let reinforcement_delta = if existed { 1 } else { 0 };
    conn.execute(
        "INSERT INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch)
         VALUES (?1, ?2, 1, ?3, ?4, ?5)
         ON CONFLICT(memory_id) DO UPDATE SET
           confidence = MAX(memory_lessons.confidence, excluded.confidence),
           reinforcement_count = memory_lessons.reinforcement_count + ?6,
           source_evidence = COALESCE(excluded.source_evidence, memory_lessons.source_evidence),
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           stale_after_epoch = excluded.stale_after_epoch",
        params![
            memory_id,
            confidence,
            req.source_evidence,
            now,
            req.stale_after_epoch,
            reinforcement_delta
        ],
    )?;
    Ok(())
}

pub fn get_lesson_metadata(conn: &Connection, memory_id: i64) -> Result<Option<LessonMetadata>> {
    conn.query_row(
        "SELECT memory_id, confidence, reinforcement_count, source_evidence,
                last_reinforced_at_epoch, stale_after_epoch
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
                l.last_reinforced_at_epoch, l.stale_after_epoch
         FROM memories m
         JOIN memory_lessons l ON l.memory_id = m.id
         WHERE m.memory_type = 'lesson'
           AND m.status = 'active'
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
           l.last_reinforced_at_epoch DESC
         LIMIT ?5",
        cols = prefixed_memory_cols("m")
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
    })
}

#[cfg(test)]
mod tests;
