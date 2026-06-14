use anyhow::{anyhow, Context, Result};
use rusqlite::{types::ValueRef, Connection, OpenFlags, Row};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::cli::types::ImportAction;

pub(in crate::cli) fn run_import(action: ImportAction) -> Result<()> {
    match action {
        ImportAction::Backup {
            source,
            best_effort,
        } => run_import_backup(source, best_effort),
    }
}

fn run_import_backup(source: PathBuf, best_effort: bool) -> Result<()> {
    if !best_effort {
        anyhow::bail!("backup import currently only supports --best-effort mode.");
    }
    let conn = crate::db::open_db().context("open runtime database for import target")?;
    let stats = import_memories_into_runtime(&source, &conn)
        .with_context(|| format!("import from {}", source.display()))?;
    println!(
        "Imported {} memories ({} skipped). Created {} workspaces, {} projects.",
        stats.memories_imported,
        stats.memories_skipped,
        stats.workspaces_created,
        stats.projects_created,
    );
    Ok(())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ImportStats {
    memories_imported: usize,
    memories_skipped: usize,
    workspaces_created: usize,
    projects_created: usize,
}

fn import_memories_into_runtime(
    source_path: &std::path::Path,
    conn: &Connection,
) -> Result<ImportStats> {
    if !source_path.exists() {
        return Err(anyhow!(
            "source database not found at {}",
            source_path.display()
        ));
    }

    let mut source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open source {}", source_path.display()))?;
    let has_cipher_key = crate::db::apply_cipher_key_if_available(&source)
        .with_context(|| format!("unlock source {}", source_path.display()))?;
    if has_cipher_key && !crate::db::can_read_schema(&source) {
        drop(source);
        source = Connection::open_with_flags(source_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("reopen unencrypted source {}", source_path.display()))?;
    }

    let columns = table_columns(&source, "memories")?;
    let session_expr = optional_column_or_literal(&columns, "session_id", "NULL");
    let files_expr = optional_column_or_literal(&columns, "files", "NULL");
    let branch_expr = optional_column_or_literal(&columns, "branch", "NULL");
    let scope_expr = optional_column_or_literal(&columns, "scope", "'project'");
    let status_expr = optional_column_or_literal(&columns, "status", "'active'");
    let reference_time_expr = optional_column_or_literal(&columns, "reference_time_epoch", "NULL");
    let query = format!(
        "SELECT id, {session_expr}, project, topic_key, title, content, memory_type, {files_expr},
                created_at_epoch, updated_at_epoch, {status_expr}, {branch_expr}, {scope_expr},
                {reference_time_expr}
         FROM memories"
    );
    let mut stmt = source.prepare(&query)?;
    let mut rows = stmt.query([])?;

    let mut stats = ImportStats::default();
    while let Some(row) = rows.next()? {
        let source_id: i64 = row.get(0)?;
        let session_id: Option<String> = row.get(1)?;
        let project: String = row.get(2)?;
        let topic_key = read_optional_text_column(row, 3, source_id, "topic_key")?;
        let title: String = row.get(4)?;
        let content: String = row.get(5)?;
        let memory_type: String = row.get(6)?;
        let files: Option<String> = row.get(7)?;
        let created_at: i64 = row.get(8)?;
        let updated_at: i64 = row.get(9)?;
        let status: String = row
            .get::<_, Option<String>>(10)?
            .unwrap_or_else(|| "active".to_string());
        let branch: Option<String> = row.get(11)?;
        let scope: String = row
            .get::<_, Option<String>>(12)?
            .unwrap_or_else(|| "project".to_string());
        let reference_time = row.get::<_, Option<i64>>(13)?.or(Some(created_at));
        let topic_key = topic_key.unwrap_or_else(|| format!("imported-{source_id}"));

        if title.is_empty() && content.is_empty() {
            stats.memories_skipped += 1;
            continue;
        }
        if let Err(error) =
            validate_import_reference_time(source_id, &title, &content, reference_time)
        {
            crate::log::warn("import", &error.to_string());
            stats.memories_skipped += 1;
            continue;
        }
        if runtime_memory_exists(conn, &project, &topic_key, &scope)? {
            stats.memories_skipped += 1;
            continue;
        }
        let result = insert_imported_memory(
            conn,
            session_id,
            &project,
            &topic_key,
            &title,
            &content,
            &memory_type,
            files,
            created_at,
            updated_at,
            reference_time.unwrap_or(created_at),
            &status,
            branch,
            &scope,
        );
        match result {
            Ok(_memory_id) => {
                stats.memories_imported += 1;
            }
            Err(error) => {
                crate::log::warn(
                    "import",
                    &format!("skipped source memory id={source_id}: {error}"),
                );
                stats.memories_skipped += 1;
            }
        }
    }

    Ok(stats)
}

fn table_columns(conn: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = HashSet::new();
    for row in rows {
        columns.insert(row?);
    }
    Ok(columns)
}

fn optional_column_or_literal(columns: &HashSet<String>, column: &str, literal: &str) -> String {
    if columns.contains(column) {
        column.to_string()
    } else {
        format!("{literal} AS {column}")
    }
}

fn read_optional_text_column(
    row: &Row<'_>,
    column_index: usize,
    source_id: i64,
    column_name: &str,
) -> Result<Option<String>> {
    match row.get_ref(column_index)? {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(value) => Ok(Some(value.to_string())),
            Err(error) => {
                crate::log::warn(
                    "import",
                    &format!(
                        "source memory id={source_id} has invalid UTF-8 {column_name}; synthesizing imported topic key: {error}"
                    ),
                );
                Ok(None)
            }
        },
        ValueRef::Integer(_) | ValueRef::Real(_) | ValueRef::Blob(_) => {
            crate::log::warn(
                "import",
                &format!(
                    "source memory id={source_id} has non-text {column_name}; synthesizing imported topic key"
                ),
            );
            Ok(None)
        }
    }
}

fn validate_import_reference_time(
    source_id: i64,
    title: &str,
    content: &str,
    reference_time_epoch: Option<i64>,
) -> Result<()> {
    let has_relative_time = crate::memory::reference_time::contains_relative_time_reference(title)
        || crate::memory::reference_time::contains_relative_time_reference(content);
    if has_relative_time && reference_time_epoch.is_none_or(|epoch| epoch <= 0) {
        anyhow::bail!(
            "skipped source memory id={source_id}: relative dates require a positive reference_time_epoch"
        );
    }
    Ok(())
}

fn runtime_memory_exists(
    conn: &Connection,
    project: &str,
    topic_key: &str,
    scope: &str,
) -> Result<bool> {
    let result = conn.query_row(
        "SELECT id FROM memories
         WHERE project = ?1 AND topic_key = ?2 AND scope = ?3
         LIMIT 1",
        rusqlite::params![project, topic_key, scope],
        |row| row.get::<_, i64>(0),
    );
    match result {
        Ok(_) => Ok(true),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_imported_memory(
    conn: &Connection,
    session_id: Option<String>,
    project: &str,
    topic_key: &str,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<String>,
    created_at: i64,
    updated_at: i64,
    reference_time_epoch: i64,
    status: &str,
    branch: Option<String>,
    scope: &str,
) -> Result<i64> {
    conn.execute_batch("SAVEPOINT remem_import_memory")?;
    let result = (|| -> Result<i64> {
        let search_context = crate::memory::search_context::build_search_context(
            memory_type,
            Some(topic_key),
            content,
            files.as_deref(),
        );
        conn.execute(
            "INSERT INTO memories
             (session_id, project, topic_key, title, content, memory_type, files, search_context,
              created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                session_id,
                project,
                topic_key,
                title,
                content,
                memory_type,
                files,
                search_context,
                created_at,
                updated_at,
                reference_time_epoch,
                status,
                branch,
                scope
            ],
        )?;
        let memory_id = conn.last_insert_rowid();
        refresh_imported_memory_entities(conn, memory_id, title, content);
        crate::retrieval::vector::upsert_memory_embedding_for_row(conn, memory_id)?;
        Ok(memory_id)
    })();

    match result {
        Ok(memory_id) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_import_memory")?;
            Ok(memory_id)
        }
        Err(error) => {
            if let Err(rollback_error) = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_import_memory; RELEASE SAVEPOINT remem_import_memory",
            ) {
                return Err(rollback_error)
                    .context(format!("rollback imported memory after failure: {error}"));
            }
            Err(error)
        }
    }
}

fn refresh_imported_memory_entities(conn: &Connection, id: i64, title: &str, content: &str) {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    if let Err(error) = crate::retrieval::entity::refresh_memory_entities(conn, id, &entities) {
        crate::log::warn(
            "import",
            &format!("entity link failed for imported memory id={id}: {error}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path, ScopedTestDataDir};
    use crate::memory::service::{search_memories, SearchRequest};
    use rusqlite::Connection;
    use std::path::Path;

    fn create_source_db(path: &Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE memories (
                id INTEGER PRIMARY KEY,
                session_id TEXT,
                project TEXT NOT NULL,
                topic_key TEXT,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                files TEXT,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                branch TEXT,
                scope TEXT DEFAULT 'project'
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn backup_import_synthesizes_malformed_topic_key_and_continues() -> Result<()> {
        let _data_dir = ScopedTestDataDir::new("import-malformed-topic-key");
        let source_path = unique_temp_db_path("runtime-import-malformed-topic-key");
        let source = create_source_db(&source_path);
        let project = "/tmp/remem-import-malformed-topic-key";
        source.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (1, 's1', ?1, x'0102',
                     'Malformed topic memory',
                     'Malformed topic key should synthesize a stable import key',
                     'discovery', NULL, 100, 200, 'active', NULL, 'project')",
            [project],
        )?;
        source.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (2, 's1', ?1, 'normal-topic',
                     'Later normal memory',
                     'Import should continue after malformed topic key',
                     'discovery', NULL, 100, 200, 'active', NULL, 'project')",
            [project],
        )?;
        drop(source);

        let runtime_conn = crate::db::open_db()?;
        let stats = import_memories_into_runtime(&source_path, &runtime_conn)?;
        assert_eq!(stats.memories_imported, 2);
        assert_eq!(stats.memories_skipped, 0);

        let synthesized_topic_key: String = runtime_conn.query_row(
            "SELECT topic_key FROM memories WHERE title = 'Malformed topic memory'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(synthesized_topic_key, "imported-1");

        let later_topic_key: String = runtime_conn.query_row(
            "SELECT topic_key FROM memories WHERE title = 'Later normal memory'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(later_topic_key, "normal-topic");

        cleanup_temp_db_files(&source_path);
        Ok(())
    }

    #[test]
    fn backup_import_writes_to_runtime_store_visible_to_search() -> Result<()> {
        let _data_dir = ScopedTestDataDir::new("import-runtime-visible");
        let source_path = unique_temp_db_path("runtime-import-source");
        let source = create_source_db(&source_path);
        let project = "/tmp/remem-import-runtime";
        source.execute(
            "INSERT INTO memories
                 (id, session_id, project, topic_key, title, content, memory_type, files,
                  created_at_epoch, updated_at_epoch, status, branch, scope)
                 VALUES (1, 's1', ?1, 'import-runtime-topic',
                         'Imported runtime memory',
                         'Imported memory should be visible from runtime search',
                         'decision', NULL, 100, 200, 'active', NULL, 'project')",
            [project],
        )?;
        drop(source);

        let runtime_conn = crate::db::open_db()?;
        let stats = import_memories_into_runtime(&source_path, &runtime_conn)?;
        assert_eq!(stats.memories_imported, 1);
        let embedding_count: i64 = runtime_conn.query_row(
            "SELECT COUNT(*)
                 FROM memory_embeddings e
                 JOIN memories m ON m.id = e.memory_id
                 WHERE m.topic_key = 'import-runtime-topic'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(embedding_count, 1);

        let results = search_memories(
            &runtime_conn,
            &SearchRequest {
                query: Some("Imported runtime memory".to_string()),
                project: Some(project.to_string()),
                memory_type: None,
                limit: 10,
                offset: 0,
                include_stale: false,
                branch: None,
                multi_hop: false,
                explain: false,
            },
        )?;

        assert_eq!(results.memories.len(), 1);
        assert_eq!(results.memories[0].title, "Imported runtime memory");
        let reference_time: i64 = runtime_conn.query_row(
            "SELECT reference_time_epoch
             FROM memories
             WHERE topic_key = 'import-runtime-topic'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(reference_time, 100);

        cleanup_temp_db_files(&source_path);
        Ok(())
    }

    #[test]
    fn backup_import_preserves_explicit_reference_time() -> Result<()> {
        let _data_dir = ScopedTestDataDir::new("import-reference-time");
        let source_path = unique_temp_db_path("runtime-import-reference-time");
        let source = create_source_db(&source_path);
        source.execute(
            "ALTER TABLE memories ADD COLUMN reference_time_epoch INTEGER",
            [],
        )?;
        let project = "/tmp/remem-import-reference-time";
        source.execute(
            "INSERT INTO memories
                 (id, session_id, project, topic_key, title, content, memory_type, files,
                  created_at_epoch, updated_at_epoch, status, branch, scope, reference_time_epoch)
                 VALUES (1, 's1', ?1, 'historical-topic',
                         'Imported historical memory',
                         'Yesterday referred to the old episode date.',
                         'decision', NULL, 200, 300, 'active', NULL, 'project', 100)",
            [project],
        )?;
        drop(source);

        let runtime_conn = crate::db::open_db()?;
        let stats = import_memories_into_runtime(&source_path, &runtime_conn)?;
        assert_eq!(stats.memories_imported, 1);
        let row: (i64, i64) = runtime_conn.query_row(
            "SELECT created_at_epoch, reference_time_epoch
             FROM memories
             WHERE topic_key = 'historical-topic'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(row, (200, 100));

        cleanup_temp_db_files(&source_path);
        Ok(())
    }

    #[test]
    fn backup_import_skips_relative_dates_without_valid_reference_time() -> Result<()> {
        let _data_dir = ScopedTestDataDir::new("import-relative-missing-reference");
        let source_path = unique_temp_db_path("runtime-import-relative-missing-reference");
        let source = create_source_db(&source_path);
        let project = "/tmp/remem-import-relative-missing-reference";
        source.execute(
            "INSERT INTO memories
                 (id, session_id, project, topic_key, title, content, memory_type, files,
                  created_at_epoch, updated_at_epoch, status, branch, scope)
                 VALUES (1, 's1', ?1, 'relative-topic',
                         'Yesterday decision',
                         'Yesterday we changed the ingestion boundary.',
                         'decision', NULL, 0, 200, 'active', NULL, 'project')",
            [project],
        )?;
        source.execute(
            "INSERT INTO memories
                 (id, session_id, project, topic_key, title, content, memory_type, files,
                  created_at_epoch, updated_at_epoch, status, branch, scope)
                 VALUES (2, 's1', ?1, 'relative-cn-topic',
                         '上个月 decision',
                         '上个月我们调整了 ingestion boundary.',
                         'decision', NULL, 0, 200, 'active', NULL, 'project')",
            [project],
        )?;
        drop(source);

        let runtime_conn = crate::db::open_db()?;
        let stats = import_memories_into_runtime(&source_path, &runtime_conn)?;
        assert_eq!(stats.memories_imported, 0);
        assert_eq!(stats.memories_skipped, 2);

        cleanup_temp_db_files(&source_path);
        Ok(())
    }

    #[test]
    fn backup_import_refuses_plaintext_target_without_override() {
        let _data_dir = ScopedTestDataDir::new("import-fail-closed-target");
        std::env::remove_var("REMEM_ALLOW_PLAINTEXT_DB");
        let source_path = unique_temp_db_path("runtime-import-fail-closed");
        let source = create_source_db(&source_path);
        drop(source);

        let err = match run_import_backup(source_path.clone(), true) {
            Ok(()) => panic!("import target must not open plaintext without explicit override"),
            Err(err) => err,
        };
        let message = format!("{err:#}");
        assert!(message.contains("open runtime database"), "got: {message}");
        assert!(message.contains("SQLCipher key"), "got: {message}");

        cleanup_temp_db_files(&source_path);
    }
}
