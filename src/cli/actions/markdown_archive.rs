mod format;
mod persist;

use anyhow::Context;
use anyhow::Result;
use format::{
    markdown_file_name, markdown_files, normalized_topic_key, parse_markdown_memory,
    render_markdown_memory, synthesized_markdown_topic_key, validate_markdown_metadata,
};
use persist::{
    markdown_ownership, update_optional_memory_provenance, upsert_markdown_lesson_metadata,
    MarkdownOwnership,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const EXPORT_VERSION: u32 = 1;
const META_START: &str = "<!-- remem-metadata-start -->";
const META_END: &str = "<!-- remem-metadata-end -->";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MarkdownMemoryMetadata {
    remem_export_version: u32,
    source_id: Option<i64>,
    project: String,
    topic_key: Option<String>,
    title: String,
    memory_type: String,
    files: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    reference_time_epoch: Option<i64>,
    status: String,
    branch: Option<String>,
    scope: String,
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
    lesson: Option<MarkdownLessonMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct MarkdownLessonMetadata {
    confidence: f64,
    reinforcement_count: i64,
    source_evidence: Option<String>,
    last_reinforced_at_epoch: i64,
    stale_after_epoch: Option<i64>,
    outcome_kind: String,
    success_count: i64,
    failure_count: i64,
    recovery_count: i64,
    correction_count: i64,
    revert_count: i64,
}

impl From<crate::memory::lesson::LessonMetadata> for MarkdownLessonMetadata {
    fn from(metadata: crate::memory::lesson::LessonMetadata) -> Self {
        Self {
            confidence: metadata.confidence,
            reinforcement_count: metadata.reinforcement_count,
            source_evidence: metadata.source_evidence,
            last_reinforced_at_epoch: metadata.last_reinforced_at_epoch,
            stale_after_epoch: metadata.stale_after_epoch,
            outcome_kind: metadata.outcome_kind,
            success_count: metadata.success_count,
            failure_count: metadata.failure_count,
            recovery_count: metadata.recovery_count,
            correction_count: metadata.correction_count,
            revert_count: metadata.revert_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct MarkdownMemoryDocument {
    metadata: MarkdownMemoryMetadata,
    content: String,
}

struct ExistingMarkdownMemory {
    title: String,
    content: String,
    memory_type: String,
    files: Option<String>,
    reference_time_epoch: Option<i64>,
    status: String,
    branch: Option<String>,
    scope: String,
    updated_at_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli) struct MarkdownExportStats {
    pub exported: usize,
    pub output: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli) struct MarkdownImportStats {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub source: PathBuf,
}

pub(in crate::cli) fn run_export_markdown(
    markdown: bool,
    output: &Path,
    project: &str,
    include_inactive: bool,
    limit: i64,
) -> Result<()> {
    if !markdown {
        anyhow::bail!("export currently supports only --markdown");
    }
    let conn = crate::db::open_db().context("open runtime database for markdown export")?;
    let stats = export_markdown_archive(
        &conn,
        MarkdownExportRequest {
            output,
            project,
            include_inactive,
            limit,
        },
    )?;
    println!(
        "Exported {} memories to {} as markdown.",
        stats.exported,
        stats.output.display()
    );
    Ok(())
}

pub(in crate::cli) fn run_import_markdown(source: &Path, best_effort: bool) -> Result<()> {
    let conn = crate::db::open_db().context("open runtime database for markdown import target")?;
    let stats = import_markdown_archive(&conn, source, best_effort)?;
    println!(
        "Imported {} markdown memories, updated {} ({} skipped) from {}.",
        stats.imported,
        stats.updated,
        stats.skipped,
        stats.source.display()
    );
    Ok(())
}

struct MarkdownExportRequest<'a> {
    output: &'a Path,
    project: &'a str,
    include_inactive: bool,
    limit: i64,
}

fn export_markdown_archive(
    conn: &Connection,
    request: MarkdownExportRequest<'_>,
) -> Result<MarkdownExportStats> {
    if request.limit <= 0 {
        anyhow::bail!("export limit must be positive");
    }
    fs::create_dir_all(request.output)
        .with_context(|| format!("create export directory {}", request.output.display()))?;
    ensure_empty_export_directory(request.output)?;
    let memories = load_export_memories(
        conn,
        request.project,
        request.include_inactive,
        request.limit,
    )?;
    for memory in &memories {
        let path = request.output.join(markdown_file_name(memory));
        fs::write(&path, render_markdown_memory(memory))
            .with_context(|| format!("write markdown memory {}", path.display()))?;
    }
    Ok(MarkdownExportStats {
        exported: memories.len(),
        output: request.output.to_path_buf(),
    })
}

fn ensure_empty_export_directory(output: &Path) -> Result<()> {
    let mut entries = fs::read_dir(output)
        .with_context(|| format!("read export directory {}", output.display()))?;
    if let Some(entry) = entries.next() {
        let entry = entry?;
        anyhow::bail!(
            "export output directory must be empty to avoid overwriting markdown edits: {} contains {}",
            output.display(),
            entry.path().display()
        );
    }
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn import_markdown_archive(
    conn: &Connection,
    source: &Path,
    best_effort: bool,
) -> Result<MarkdownImportStats> {
    let files = markdown_files(source)?;
    let mut stats = MarkdownImportStats {
        imported: 0,
        updated: 0,
        skipped: 0,
        source: source.to_path_buf(),
    };
    for file in files {
        match import_markdown_file(conn, &file) {
            Ok(ImportFileOutcome::Imported) => stats.imported += 1,
            Ok(ImportFileOutcome::Updated) => stats.updated += 1,
            Err(error) if best_effort => {
                crate::log::warn(
                    "import",
                    &format!("skipped markdown memory {}: {error}", file.display()),
                );
                stats.skipped += 1;
            }
            Err(error) => return Err(error).with_context(|| format!("import {}", file.display())),
        }
    }
    Ok(stats)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportFileOutcome {
    Imported,
    Updated,
}

fn import_markdown_file(conn: &Connection, path: &Path) -> Result<ImportFileOutcome> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read markdown memory {}", path.display()))?;
    let doc = parse_markdown_memory(&raw)?;
    validate_markdown_metadata(&doc)?;
    let metadata_topic_key = normalized_topic_key(doc.metadata.topic_key.as_deref());
    if let Some(existing_id) = runtime_memory_id_by_source(conn, &doc)? {
        update_markdown_memory(conn, existing_id, &doc, metadata_topic_key.as_deref())?;
        return Ok(ImportFileOutcome::Updated);
    }
    let topic_key = metadata_topic_key
        .unwrap_or_else(|| synthesized_markdown_topic_key(path, &doc.metadata.title));
    if let Some(existing_id) =
        runtime_memory_id(conn, &doc.metadata.project, &topic_key, &doc.metadata.scope)?
    {
        update_markdown_memory(conn, existing_id, &doc, Some(&topic_key))?;
        return Ok(ImportFileOutcome::Updated);
    }
    insert_markdown_memory(conn, &doc, Some(&topic_key))?;
    Ok(ImportFileOutcome::Imported)
}

fn load_export_memories(
    conn: &Connection,
    project: &str,
    include_inactive: bool,
    limit: i64,
) -> Result<Vec<MarkdownMemoryDocument>> {
    let status_filter =
        crate::memory::memory_current_filter_sql("status", "expires_at_epoch", include_inactive);
    let evidence_expr = if column_exists(conn, "memories", "evidence_event_ids")? {
        "evidence_event_ids"
    } else {
        "NULL AS evidence_event_ids"
    };
    let source_candidate_expr = if column_exists(conn, "memories", "source_candidate_id")? {
        "source_candidate_id"
    } else {
        "NULL AS source_candidate_id"
    };
    let sql = format!(
        "SELECT id, project, topic_key, title, content, memory_type, files,
                created_at_epoch, updated_at_epoch, reference_time_epoch,
                status, branch, scope, source_project, target_project,
                owner_scope, owner_key, topic_domain, routing_confidence,
                routing_reason, context_class, expires_at_epoch, valid_from_epoch,
                valid_to_epoch, {evidence_expr}, {source_candidate_expr}
         FROM memories
         WHERE (project = ?1 OR scope = 'global')
           AND {status_filter}
         ORDER BY memory_type, updated_at_epoch DESC, id
         LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![project, limit], |row| {
        Ok(MarkdownMemoryDocument {
            metadata: MarkdownMemoryMetadata {
                remem_export_version: EXPORT_VERSION,
                source_id: Some(row.get(0)?),
                project: row.get(1)?,
                topic_key: row.get(2)?,
                title: row.get(3)?,
                memory_type: row.get(5)?,
                files: row.get(6)?,
                created_at_epoch: row.get(7)?,
                updated_at_epoch: row.get(8)?,
                reference_time_epoch: row.get(9)?,
                status: row.get(10)?,
                branch: row.get(11)?,
                scope: row
                    .get::<_, Option<String>>(12)?
                    .unwrap_or_else(|| "project".to_string()),
                source_project: row.get(13)?,
                target_project: row.get(14)?,
                owner_scope: row.get(15)?,
                owner_key: row.get(16)?,
                topic_domain: row.get(17)?,
                routing_confidence: row.get(18)?,
                routing_reason: row.get(19)?,
                context_class: row.get(20)?,
                expires_at_epoch: row.get(21)?,
                valid_from_epoch: row.get(22)?,
                valid_to_epoch: row.get(23)?,
                evidence_event_ids: row.get(24)?,
                source_candidate_id: row.get(25)?,
                lesson: None,
            },
            content: row.get(4)?,
        })
    })?;
    let mut docs = crate::db::query::collect_rows(rows)?;
    for doc in &mut docs {
        if doc.metadata.memory_type == crate::memory::MemoryType::Lesson.as_str() {
            if let Some(source_id) = doc.metadata.source_id {
                doc.metadata.lesson = crate::memory::lesson::get_lesson_metadata(conn, source_id)?
                    .map(MarkdownLessonMetadata::from);
            }
        }
    }
    Ok(docs)
}

fn runtime_memory_id_by_source(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
) -> Result<Option<i64>> {
    let Some(source_id) = doc.metadata.source_id else {
        return Ok(None);
    };
    let result = conn.query_row(
        "SELECT id FROM memories
         WHERE id = ?1 AND project = ?2 AND scope = ?3
         LIMIT 1",
        rusqlite::params![source_id, doc.metadata.project, doc.metadata.scope],
        |row| row.get::<_, i64>(0),
    );
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn runtime_memory_id(
    conn: &Connection,
    project: &str,
    topic_key: &str,
    scope: &str,
) -> Result<Option<i64>> {
    let result = conn.query_row(
        "SELECT id FROM memories
         WHERE project = ?1 AND topic_key = ?2 AND scope = ?3
         LIMIT 1",
        rusqlite::params![project, topic_key, scope],
        |row| row.get::<_, i64>(0),
    );
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn update_markdown_memory(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
) -> Result<()> {
    conn.execute_batch("SAVEPOINT remem_update_markdown_memory")?;
    let result = (|| -> Result<()> {
        let reference_time_epoch = doc
            .metadata
            .reference_time_epoch
            .unwrap_or(doc.metadata.created_at_epoch);
        let existing = load_existing_markdown_memory(conn, memory_id)?;
        let updated_at_epoch = effective_update_epoch(&existing, doc, reference_time_epoch);
        let search_context = crate::memory::search_context::build_search_context(
            &doc.metadata.memory_type,
            topic_key,
            &doc.content,
            doc.metadata.files.as_deref(),
        );
        let ownership = markdown_ownership(doc);
        conn.execute(
            "UPDATE memories
             SET session_id = NULL,
                 project = ?1,
                 topic_key = ?2,
                 title = ?3,
                 content = ?4,
                 memory_type = ?5,
                 files = ?6,
                 search_context = ?7,
                 created_at_epoch = ?8,
                 updated_at_epoch = ?9,
                 reference_time_epoch = ?10,
                 status = ?11,
                 branch = ?12,
                 scope = ?13,
                 source_project = ?14,
                 target_project = ?15,
                 owner_scope = ?16,
                 owner_key = ?17,
                 topic_domain = ?18,
                 routing_confidence = ?19,
                 routing_reason = ?20,
                 context_class = ?21,
                 expires_at_epoch = ?22,
                 valid_from_epoch = ?23,
                 valid_to_epoch = ?24
             WHERE id = ?25",
            rusqlite::params![
                doc.metadata.project,
                topic_key,
                doc.metadata.title,
                doc.content,
                doc.metadata.memory_type,
                doc.metadata.files,
                search_context,
                doc.metadata.created_at_epoch,
                updated_at_epoch,
                reference_time_epoch,
                doc.metadata.status,
                doc.metadata.branch,
                doc.metadata.scope,
                ownership.source_project,
                ownership.target_project,
                ownership.owner_scope,
                ownership.owner_key,
                doc.metadata.topic_domain,
                doc.metadata.routing_confidence,
                doc.metadata.routing_reason,
                ownership.context_class,
                doc.metadata.expires_at_epoch,
                doc.metadata.valid_from_epoch,
                doc.metadata.valid_to_epoch,
                memory_id,
            ],
        )?;
        update_optional_memory_provenance(conn, memory_id, doc)?;
        refresh_markdown_memory_indexes(
            conn,
            memory_id,
            doc,
            topic_key,
            &ownership,
            updated_at_epoch,
        )?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_update_markdown_memory")?;
            Ok(())
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_update_markdown_memory; RELEASE SAVEPOINT remem_update_markdown_memory",
            ) {
                return Err(rollback_error)
                    .context(format!("rollback markdown memory update after failure: {error}"));
            }
            Err(error)
        }
    }
}

fn load_existing_markdown_memory(
    conn: &Connection,
    memory_id: i64,
) -> Result<ExistingMarkdownMemory> {
    conn.query_row(
        "SELECT title, content, memory_type, files, reference_time_epoch,
                status, branch, scope, updated_at_epoch
         FROM memories
         WHERE id = ?1",
        rusqlite::params![memory_id],
        |row| {
            Ok(ExistingMarkdownMemory {
                title: row.get(0)?,
                content: row.get(1)?,
                memory_type: row.get(2)?,
                files: row.get(3)?,
                reference_time_epoch: row.get(4)?,
                status: row.get(5)?,
                branch: row.get(6)?,
                scope: row.get(7)?,
                updated_at_epoch: row.get(8)?,
            })
        },
    )
    .with_context(|| format!("load existing markdown memory id={memory_id}"))
}

fn effective_update_epoch(
    existing: &ExistingMarkdownMemory,
    doc: &MarkdownMemoryDocument,
    reference_time_epoch: i64,
) -> i64 {
    let changed = existing.title != doc.metadata.title
        || existing.content != doc.content
        || existing.memory_type != doc.metadata.memory_type
        || existing.files != doc.metadata.files
        || existing.reference_time_epoch != Some(reference_time_epoch)
        || existing.status != doc.metadata.status
        || existing.branch != doc.metadata.branch
        || existing.scope != doc.metadata.scope;
    if changed {
        chrono::Utc::now()
            .timestamp()
            .max(existing.updated_at_epoch.saturating_add(1))
            .max(doc.metadata.updated_at_epoch)
    } else {
        existing.updated_at_epoch
    }
}

fn insert_markdown_memory(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
) -> Result<i64> {
    conn.execute_batch("SAVEPOINT remem_import_markdown_memory")?;
    let result = (|| -> Result<i64> {
        let reference_time_epoch = doc
            .metadata
            .reference_time_epoch
            .unwrap_or(doc.metadata.created_at_epoch);
        let search_context = crate::memory::search_context::build_search_context(
            &doc.metadata.memory_type,
            topic_key,
            &doc.content,
            doc.metadata.files.as_deref(),
        );
        let ownership = markdown_ownership(doc);
        conn.execute(
            "INSERT INTO memories
             (session_id, project, topic_key, title, content, memory_type, files, search_context,
              created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
              source_project, target_project, owner_scope, owner_key, topic_domain,
              routing_confidence, routing_reason, context_class, expires_at_epoch,
              valid_from_epoch, valid_to_epoch)
             VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            rusqlite::params![
                doc.metadata.project,
                topic_key,
                doc.metadata.title,
                doc.content,
                doc.metadata.memory_type,
                doc.metadata.files,
                search_context,
                doc.metadata.created_at_epoch,
                doc.metadata.updated_at_epoch,
                reference_time_epoch,
                doc.metadata.status,
                doc.metadata.branch,
                doc.metadata.scope,
                ownership.source_project,
                ownership.target_project,
                ownership.owner_scope,
                ownership.owner_key,
                doc.metadata.topic_domain,
                doc.metadata.routing_confidence,
                doc.metadata.routing_reason,
                ownership.context_class,
                doc.metadata.expires_at_epoch,
                doc.metadata.valid_from_epoch,
                doc.metadata.valid_to_epoch,
            ],
        )?;
        let memory_id = conn.last_insert_rowid();
        update_optional_memory_provenance(conn, memory_id, doc)?;
        refresh_markdown_memory_indexes(
            conn,
            memory_id,
            doc,
            topic_key,
            &ownership,
            doc.metadata.updated_at_epoch,
        )?;
        Ok(memory_id)
    })();

    match result {
        Ok(memory_id) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_import_markdown_memory")?;
            Ok(memory_id)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_import_markdown_memory; RELEASE SAVEPOINT remem_import_markdown_memory",
            ) {
                return Err(rollback_error)
                    .context(format!("rollback markdown memory import after failure: {error}"));
            }
            Err(error)
        }
    }
}

fn refresh_markdown_memory_indexes(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
    ownership: &MarkdownOwnership<'_>,
    updated_at_epoch: i64,
) -> Result<()> {
    super::import::refresh_imported_memory_entities(
        conn,
        memory_id,
        &doc.metadata.title,
        &doc.content,
    );
    let active_state_key_id = if doc.metadata.status == "active" {
        if let Some(decision) = crate::memory::state_key::derive_state_key(
            &doc.metadata.memory_type,
            topic_key,
            &doc.metadata.title,
            &doc.content,
        ) {
            Some(crate::memory::state_key::attach_current_memory(
                conn,
                memory_id,
                ownership.owner_scope,
                ownership.owner_key,
                &doc.metadata.memory_type,
                &decision,
                updated_at_epoch,
            )?)
        } else {
            None
        }
    } else {
        None
    };
    crate::memory::store::clear_obsolete_state_key_links(
        conn,
        memory_id,
        active_state_key_id,
        updated_at_epoch,
    )?;
    if doc.metadata.memory_type == crate::memory::MemoryType::Lesson.as_str() {
        upsert_markdown_lesson_metadata(conn, memory_id, doc, updated_at_epoch)?;
    }
    crate::retrieval::vector::upsert_memory_embedding_for_row(conn, memory_id)?;
    Ok(())
}

#[cfg(test)]
mod tests;
