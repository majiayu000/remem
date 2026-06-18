use super::persist::{
    default_markdown_lesson_metadata, markdown_source_candidate_id, MarkdownOwnership,
};
use super::{
    column_exists, MarkdownLessonMetadata, MarkdownMemoryDocument, MarkdownMemoryMetadata,
};
use anyhow::{Context, Result};
use rusqlite::Connection;

struct ExistingMarkdownMemory {
    project: String,
    topic_key: Option<String>,
    title: String,
    content: String,
    memory_type: String,
    files: Option<String>,
    created_at_epoch: i64,
    reference_time_epoch: Option<i64>,
    status: String,
    branch: Option<String>,
    scope: String,
    updated_at_epoch: i64,
    source_project: Option<String>,
    target_project: Option<String>,
    owner_scope: Option<String>,
    owner_key: Option<String>,
    topic_domain: Option<String>,
    routing_confidence: Option<f64>,
    routing_reason: Option<String>,
    context_class: Option<String>,
    expires_at_epoch: Option<i64>,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    evidence_event_ids: Option<String>,
    source_candidate_id: Option<i64>,
    has_evidence_event_ids: bool,
    has_source_candidate_id: bool,
    lesson: Option<MarkdownLessonMetadata>,
}

pub(super) fn markdown_update_epoch(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
    reference_time_epoch: i64,
    ownership: &MarkdownOwnership<'_>,
) -> Result<i64> {
    let existing = load_existing_markdown_memory(conn, memory_id)?;
    let source_candidate_id = if existing.has_source_candidate_id {
        markdown_source_candidate_id(conn, doc)?
    } else {
        None
    };
    Ok(effective_update_epoch(
        &existing,
        doc,
        topic_key,
        reference_time_epoch,
        ownership,
        source_candidate_id,
    ))
}

fn load_existing_markdown_memory(
    conn: &Connection,
    memory_id: i64,
) -> Result<ExistingMarkdownMemory> {
    let has_evidence_event_ids = column_exists(conn, "memories", "evidence_event_ids")?;
    let has_source_candidate_id = column_exists(conn, "memories", "source_candidate_id")?;
    let evidence_expr = if has_evidence_event_ids {
        "evidence_event_ids"
    } else {
        "NULL AS evidence_event_ids"
    };
    let source_candidate_expr = if has_source_candidate_id {
        "source_candidate_id"
    } else {
        "NULL AS source_candidate_id"
    };
    let sql = format!(
        "SELECT project, topic_key, title, content, memory_type, files,
                created_at_epoch, reference_time_epoch, status, branch, scope,
                updated_at_epoch, source_project, target_project, owner_scope,
                owner_key, topic_domain, routing_confidence, routing_reason,
                context_class, expires_at_epoch, valid_from_epoch, valid_to_epoch,
                {evidence_expr}, {source_candidate_expr}
         FROM memories
         WHERE id = ?1"
    );
    let mut existing = conn
        .query_row(&sql, rusqlite::params![memory_id], |row| {
            Ok(ExistingMarkdownMemory {
                project: row.get(0)?,
                topic_key: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                memory_type: row.get(4)?,
                files: row.get(5)?,
                created_at_epoch: row.get(6)?,
                reference_time_epoch: row.get(7)?,
                status: row.get(8)?,
                branch: row.get(9)?,
                scope: row.get(10)?,
                updated_at_epoch: row.get(11)?,
                source_project: row.get(12)?,
                target_project: row.get(13)?,
                owner_scope: row.get(14)?,
                owner_key: row.get(15)?,
                topic_domain: row.get(16)?,
                routing_confidence: row.get(17)?,
                routing_reason: row.get(18)?,
                context_class: row.get(19)?,
                expires_at_epoch: row.get(20)?,
                valid_from_epoch: row.get(21)?,
                valid_to_epoch: row.get(22)?,
                evidence_event_ids: row.get(23)?,
                source_candidate_id: row.get(24)?,
                has_evidence_event_ids,
                has_source_candidate_id,
                lesson: None,
            })
        })
        .with_context(|| format!("load existing markdown memory id={memory_id}"))?;
    if existing.memory_type == crate::memory::MemoryType::Lesson.as_str() {
        existing.lesson = crate::memory::lesson::get_lesson_metadata(conn, memory_id)?
            .map(MarkdownLessonMetadata::from);
    }
    Ok(existing)
}

fn effective_update_epoch(
    existing: &ExistingMarkdownMemory,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
    reference_time_epoch: i64,
    ownership: &MarkdownOwnership<'_>,
    source_candidate_id: Option<i64>,
) -> i64 {
    let changed = markdown_core_changed(existing, doc, topic_key, reference_time_epoch)
        || markdown_ownership_changed(existing, ownership)
        || markdown_routing_lifecycle_changed(existing, &doc.metadata)
        || markdown_provenance_changed(existing, &doc.metadata, source_candidate_id)
        || markdown_lesson_changed(existing, doc);
    if changed {
        chrono::Utc::now()
            .timestamp()
            .max(existing.updated_at_epoch.saturating_add(1))
            .max(doc.metadata.updated_at_epoch)
    } else {
        existing.updated_at_epoch
    }
}

fn markdown_core_changed(
    existing: &ExistingMarkdownMemory,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
    reference_time_epoch: i64,
) -> bool {
    existing.project != doc.metadata.project
        || existing.topic_key.as_deref() != topic_key
        || existing.title != doc.metadata.title
        || existing.content != doc.content
        || existing.memory_type != doc.metadata.memory_type
        || existing.files != doc.metadata.files
        || existing.created_at_epoch != doc.metadata.created_at_epoch
        || existing.reference_time_epoch != Some(reference_time_epoch)
        || existing.status != doc.metadata.status
        || existing.branch != doc.metadata.branch
        || existing.scope != doc.metadata.scope
}

fn markdown_ownership_changed(
    existing: &ExistingMarkdownMemory,
    ownership: &MarkdownOwnership<'_>,
) -> bool {
    existing.source_project.as_deref() != Some(ownership.source_project)
        || existing.target_project.as_deref() != ownership.target_project
        || existing.owner_scope.as_deref() != Some(ownership.owner_scope)
        || existing.owner_key.as_deref() != Some(ownership.owner_key)
        || existing.context_class.as_deref() != Some(ownership.context_class)
}

fn markdown_routing_lifecycle_changed(
    existing: &ExistingMarkdownMemory,
    metadata: &MarkdownMemoryMetadata,
) -> bool {
    existing.topic_domain != metadata.topic_domain
        || existing.routing_confidence != metadata.routing_confidence
        || existing.routing_reason != metadata.routing_reason
        || existing.expires_at_epoch != metadata.expires_at_epoch
        || existing.valid_from_epoch != metadata.valid_from_epoch
        || existing.valid_to_epoch != metadata.valid_to_epoch
}

fn markdown_provenance_changed(
    existing: &ExistingMarkdownMemory,
    metadata: &MarkdownMemoryMetadata,
    source_candidate_id: Option<i64>,
) -> bool {
    (existing.has_evidence_event_ids && existing.evidence_event_ids != metadata.evidence_event_ids)
        || (existing.has_source_candidate_id && existing.source_candidate_id != source_candidate_id)
}

fn markdown_lesson_changed(
    existing: &ExistingMarkdownMemory,
    doc: &MarkdownMemoryDocument,
) -> bool {
    let is_lesson = doc.metadata.memory_type == crate::memory::MemoryType::Lesson.as_str();
    if !is_lesson {
        return existing.lesson.is_some();
    }
    let fallback;
    let effective_lesson = if let Some(lesson) = doc.metadata.lesson.as_ref() {
        Some(lesson)
    } else {
        fallback = default_markdown_lesson_metadata(existing.updated_at_epoch);
        Some(&fallback)
    };
    existing.lesson.as_ref() != effective_lesson
}
