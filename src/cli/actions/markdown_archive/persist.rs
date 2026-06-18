use super::{column_exists, MarkdownLessonMetadata, MarkdownMemoryDocument};
use anyhow::Result;
use rusqlite::Connection;

pub(super) struct MarkdownOwnership<'a> {
    pub(super) source_project: &'a str,
    pub(super) target_project: Option<&'a str>,
    pub(super) owner_scope: &'a str,
    pub(super) owner_key: &'a str,
    pub(super) context_class: &'a str,
}

pub(super) fn markdown_ownership(doc: &MarkdownMemoryDocument) -> MarkdownOwnership<'_> {
    let fallback =
        crate::memory::store::default_ownership(&doc.metadata.project, &doc.metadata.scope);
    MarkdownOwnership {
        source_project: doc
            .metadata
            .source_project
            .as_deref()
            .unwrap_or(fallback.source_project),
        target_project: doc
            .metadata
            .target_project
            .as_deref()
            .or(fallback.target_project),
        owner_scope: doc
            .metadata
            .owner_scope
            .as_deref()
            .unwrap_or(fallback.owner_scope),
        owner_key: doc
            .metadata
            .owner_key
            .as_deref()
            .unwrap_or(fallback.owner_key),
        context_class: doc
            .metadata
            .context_class
            .as_deref()
            .unwrap_or(fallback.context_class),
    }
}

pub(super) fn update_optional_memory_provenance(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
) -> Result<()> {
    if column_exists(conn, "memories", "evidence_event_ids")? {
        conn.execute(
            "UPDATE memories SET evidence_event_ids = ?1 WHERE id = ?2",
            rusqlite::params![doc.metadata.evidence_event_ids, memory_id],
        )?;
    }
    if column_exists(conn, "memories", "source_candidate_id")? {
        let source_candidate_id = markdown_source_candidate_id(conn, doc)?;
        conn.execute(
            "UPDATE memories SET source_candidate_id = ?1 WHERE id = ?2",
            rusqlite::params![source_candidate_id, memory_id],
        )?;
    }
    Ok(())
}

pub(super) fn markdown_source_candidate_id(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
) -> Result<Option<i64>> {
    if let Some(candidate_id) = doc.metadata.source_candidate_id {
        Ok(source_candidate_id_exists(conn, candidate_id)?.then_some(candidate_id))
    } else {
        Ok(None)
    }
}

pub(super) fn upsert_markdown_lesson_metadata(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
    updated_at_epoch: i64,
) -> Result<()> {
    let fallback = default_markdown_lesson_metadata(updated_at_epoch);
    let lesson = doc.metadata.lesson.as_ref().unwrap_or(&fallback);
    conn.execute(
        "INSERT INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch, outcome_kind,
          success_count, failure_count, recovery_count, correction_count, revert_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(memory_id) DO UPDATE SET
           confidence = excluded.confidence,
           reinforcement_count = excluded.reinforcement_count,
           source_evidence = excluded.source_evidence,
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           stale_after_epoch = excluded.stale_after_epoch,
           outcome_kind = excluded.outcome_kind,
           success_count = excluded.success_count,
           failure_count = excluded.failure_count,
           recovery_count = excluded.recovery_count,
           correction_count = excluded.correction_count,
           revert_count = excluded.revert_count",
        rusqlite::params![
            memory_id,
            lesson.confidence,
            lesson.reinforcement_count,
            lesson.source_evidence,
            lesson.last_reinforced_at_epoch,
            lesson.stale_after_epoch,
            lesson.outcome_kind,
            lesson.success_count,
            lesson.failure_count,
            lesson.recovery_count,
            lesson.correction_count,
            lesson.revert_count,
        ],
    )?;
    Ok(())
}

pub(super) fn default_markdown_lesson_metadata(updated_at_epoch: i64) -> MarkdownLessonMetadata {
    MarkdownLessonMetadata {
        confidence: 0.7,
        reinforcement_count: 1,
        source_evidence: Some("markdown_import".to_string()),
        last_reinforced_at_epoch: updated_at_epoch,
        stale_after_epoch: None,
        outcome_kind: "unknown".to_string(),
        success_count: 0,
        failure_count: 0,
        recovery_count: 0,
        correction_count: 0,
        revert_count: 0,
    }
}

fn source_candidate_id_exists(conn: &Connection, candidate_id: i64) -> Result<bool> {
    let result = conn.query_row(
        "SELECT id FROM memory_candidates WHERE id = ?1 LIMIT 1",
        rusqlite::params![candidate_id],
        |row| row.get::<_, i64>(0),
    );
    match result {
        Ok(_) => Ok(true),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(error.into()),
    }
}
