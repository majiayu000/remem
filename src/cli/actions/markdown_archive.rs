use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const EXPORT_VERSION: u32 = 1;
const META_START: &str = "<!-- remem-metadata-start -->";
const META_END: &str = "<!-- remem-metadata-end -->";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownMemoryDocument {
    metadata: MarkdownMemoryMetadata,
    content: String,
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
    let topic_key = doc
        .metadata
        .topic_key
        .clone()
        .unwrap_or_else(|| synthesized_markdown_topic_key(path, &doc.metadata.title));
    if let Some(existing_id) =
        runtime_memory_id(conn, &doc.metadata.project, &topic_key, &doc.metadata.scope)?
    {
        update_markdown_memory(conn, existing_id, &doc, &topic_key)?;
        return Ok(ImportFileOutcome::Updated);
    }
    insert_markdown_memory(conn, &doc, &topic_key)?;
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
    let sql = format!(
        "SELECT id, project, topic_key, title, content, memory_type, files,
                created_at_epoch, updated_at_epoch, reference_time_epoch,
                status, branch, scope
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
            },
            content: row.get(4)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

fn render_markdown_memory(doc: &MarkdownMemoryDocument) -> String {
    let metadata = serde_json::to_string_pretty(&doc.metadata)
        .expect("markdown export metadata should serialize");
    format!(
        "{META_START}\n{metadata}\n{META_END}\n\n# {}\n\n{}",
        heading_title(&doc.metadata.title),
        doc.content
    )
}

fn parse_markdown_memory(raw: &str) -> Result<MarkdownMemoryDocument> {
    let body = raw
        .strip_prefix(META_START)
        .ok_or_else(|| anyhow!("missing remem markdown metadata start marker"))?;
    let end_marker = format!("\n{META_END}");
    let end = body
        .find(&end_marker)
        .ok_or_else(|| anyhow!("missing remem markdown metadata end marker"))?;
    let metadata: MarkdownMemoryMetadata =
        serde_json::from_str(body[..end].trim()).context("parse remem markdown metadata")?;
    let content_start = end + end_marker.len();
    let content = strip_generated_heading(&body[content_start..], &metadata.title);
    Ok(MarkdownMemoryDocument { metadata, content })
}

fn validate_markdown_metadata(doc: &MarkdownMemoryDocument) -> Result<()> {
    if doc.metadata.remem_export_version != EXPORT_VERSION {
        anyhow::bail!(
            "unsupported remem markdown export version {}",
            doc.metadata.remem_export_version
        );
    }
    if crate::memory::MemoryType::parse(&doc.metadata.memory_type).is_none() {
        anyhow::bail!("unsupported memory_type {}", doc.metadata.memory_type);
    }
    if doc.metadata.project.trim().is_empty() {
        anyhow::bail!("markdown memory project must not be empty");
    }
    if doc.metadata.title.trim().is_empty() {
        anyhow::bail!("markdown memory title must not be empty");
    }
    if doc.content.trim().is_empty() {
        anyhow::bail!("markdown memory content must not be empty");
    }
    if !matches!(
        doc.metadata.status.as_str(),
        "active" | "stale" | "archived"
    ) {
        anyhow::bail!("unsupported markdown memory status {}", doc.metadata.status);
    }
    if !matches!(doc.metadata.scope.as_str(), "project" | "global") {
        anyhow::bail!("unsupported markdown memory scope {}", doc.metadata.scope);
    }
    let reference_time = doc
        .metadata
        .reference_time_epoch
        .or(Some(doc.metadata.created_at_epoch));
    validate_reference_time(
        doc.metadata.source_id,
        &doc.metadata.title,
        &doc.content,
        reference_time,
    )
}

fn strip_generated_heading(raw: &str, title: &str) -> String {
    let mut content = raw.trim_start_matches('\n');
    let Some(after_hash) = content.strip_prefix("# ") else {
        return content.to_string();
    };
    let Some(line_end) = after_hash.find('\n') else {
        return String::new();
    };
    let heading = &after_hash[..line_end];
    if heading != heading_title(title) {
        return content.to_string();
    }
    content = &after_hash[line_end + 1..];
    content.trim_start_matches('\n').to_string()
}

fn heading_title(title: &str) -> String {
    title
        .lines()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('#', "\\#")
}

fn markdown_file_name(doc: &MarkdownMemoryDocument) -> String {
    let source_id = doc.metadata.source_id.unwrap_or_default();
    let label = doc
        .metadata
        .topic_key
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&doc.metadata.title);
    format!(
        "{source_id:06}-{}-{}.md",
        slug_component(&doc.metadata.memory_type),
        slug_component(label)
    )
}

fn slug_component(value: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "memory".to_string()
    } else {
        slug.chars().take(80).collect()
    }
}

fn markdown_files(source: &Path) -> Result<Vec<PathBuf>> {
    if source.is_file() {
        return Ok(source
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            .then(|| vec![source.to_path_buf()])
            .unwrap_or_default());
    }
    if !source.exists() {
        anyhow::bail!("markdown source not found at {}", source.display());
    }
    let mut files = Vec::new();
    collect_markdown_files(source, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_markdown_files(&path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            files.push(path);
        }
    }
    Ok(())
}

fn validate_reference_time(
    source_id: Option<i64>,
    title: &str,
    content: &str,
    reference_time_epoch: Option<i64>,
) -> Result<()> {
    let has_relative_time = crate::memory::reference_time::contains_relative_time_reference(title)
        || crate::memory::reference_time::contains_relative_time_reference(content);
    if has_relative_time && reference_time_epoch.is_none_or(|epoch| epoch <= 0) {
        let source = source_id
            .map(|id| format!("source memory id={id}"))
            .unwrap_or_else(|| "markdown memory".to_string());
        anyhow::bail!("{source}: relative dates require a positive reference_time_epoch");
    }
    Ok(())
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
    topic_key: &str,
) -> Result<()> {
    conn.execute_batch("SAVEPOINT remem_update_markdown_memory")?;
    let result = (|| -> Result<()> {
        let reference_time_epoch = doc
            .metadata
            .reference_time_epoch
            .unwrap_or(doc.metadata.created_at_epoch);
        let search_context = crate::memory::search_context::build_search_context(
            &doc.metadata.memory_type,
            Some(topic_key),
            &doc.content,
            doc.metadata.files.as_deref(),
        );
        let ownership =
            crate::memory::store::default_ownership(&doc.metadata.project, &doc.metadata.scope);
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
                 context_class = ?18
             WHERE id = ?19",
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
                ownership.context_class,
                memory_id,
            ],
        )?;
        refresh_markdown_memory_indexes(conn, memory_id, doc, topic_key, &ownership)?;
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

fn insert_markdown_memory(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
    topic_key: &str,
) -> Result<i64> {
    conn.execute_batch("SAVEPOINT remem_import_markdown_memory")?;
    let result = (|| -> Result<i64> {
        let reference_time_epoch = doc
            .metadata
            .reference_time_epoch
            .unwrap_or(doc.metadata.created_at_epoch);
        let search_context = crate::memory::search_context::build_search_context(
            &doc.metadata.memory_type,
            Some(topic_key),
            &doc.content,
            doc.metadata.files.as_deref(),
        );
        let ownership =
            crate::memory::store::default_ownership(&doc.metadata.project, &doc.metadata.scope);
        conn.execute(
            "INSERT INTO memories
             (session_id, project, topic_key, title, content, memory_type, files, search_context,
              created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
              source_project, target_project, owner_scope, owner_key, context_class)
             VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18)",
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
                ownership.context_class,
            ],
        )?;
        let memory_id = conn.last_insert_rowid();
        refresh_markdown_memory_indexes(conn, memory_id, doc, topic_key, &ownership)?;
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
    topic_key: &str,
    ownership: &crate::memory::store::DefaultOwnership<'_>,
) -> Result<()> {
    super::import::refresh_imported_memory_entities(
        conn,
        memory_id,
        &doc.metadata.title,
        &doc.content,
    );
    if doc.metadata.status == "active" {
        if let Some(decision) = crate::memory::state_key::derive_state_key(
            &doc.metadata.memory_type,
            Some(topic_key),
            &doc.metadata.title,
            &doc.content,
        ) {
            crate::memory::state_key::attach_current_memory(
                conn,
                memory_id,
                ownership.owner_scope,
                ownership.owner_key,
                &doc.metadata.memory_type,
                &decision,
                doc.metadata.updated_at_epoch,
            )?;
        }
    }
    if doc.metadata.memory_type == crate::memory::MemoryType::Lesson.as_str() {
        insert_default_lesson_metadata(conn, memory_id, doc.metadata.updated_at_epoch)?;
    }
    crate::retrieval::vector::upsert_memory_embedding_for_row(conn, memory_id)?;
    Ok(())
}

fn insert_default_lesson_metadata(
    conn: &Connection,
    memory_id: i64,
    updated_at_epoch: i64,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch, outcome_kind,
          success_count, failure_count, recovery_count, correction_count, revert_count)
         VALUES (?1, 0.7, 1, 'markdown_import', ?2, NULL, 'unknown', 0, 0, 0, 0, 0)",
        rusqlite::params![memory_id, updated_at_epoch],
    )?;
    Ok(())
}

fn synthesized_markdown_topic_key(path: &Path, title: &str) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(title);
    format!("markdown-{}", slug_component(stem))
}

#[cfg(test)]
mod tests;
