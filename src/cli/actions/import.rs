use anyhow::{anyhow, Context, Result};
use rusqlite::{Connection, OpenFlags};
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
    let query = format!(
        "SELECT id, {session_expr}, project, topic_key, title, content, memory_type, {files_expr},
                created_at_epoch, updated_at_epoch, {status_expr}, {branch_expr}, {scope_expr}
         FROM memories"
    );
    let mut stmt = source.prepare(&query)?;
    let mut rows = stmt.query([])?;

    let mut stats = ImportStats::default();
    while let Some(row) = rows.next()? {
        let source_id: i64 = row.get(0)?;
        let session_id: Option<String> = row.get(1)?;
        let project: String = row.get(2)?;
        let topic_key: Option<String> = row.get(3)?;
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
        let topic_key = topic_key.unwrap_or_else(|| format!("imported-{source_id}"));

        if title.is_empty() && content.is_empty() {
            stats.memories_skipped += 1;
            continue;
        }
        if runtime_memory_exists(conn, &project, &topic_key, &scope)? {
            stats.memories_skipped += 1;
            continue;
        }
        let search_context = crate::memory::search_context::build_search_context(
            &memory_type,
            Some(&topic_key),
            &content,
            files.as_deref(),
        );

        let result = conn.execute(
            "INSERT INTO memories
             (session_id, project, topic_key, title, content, memory_type, files, search_context,
              created_at_epoch, updated_at_epoch, status, branch, scope)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                status,
                branch,
                scope
            ],
        );
        match result {
            Ok(_) => {
                stats.memories_imported += 1;
                let memory_id = conn.last_insert_rowid();
                refresh_imported_memory_entities(conn, memory_id, &title, &content);
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

fn refresh_imported_memory_entities(conn: &Connection, id: i64, title: &str, content: &str) {
    let entities = crate::retrieval::entity::extract_entities(title, content);
    if entities.is_empty() {
        return;
    }
    if let Err(error) = crate::retrieval::entity::link_entities(conn, id, &entities) {
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
    fn backup_import_writes_to_runtime_store_visible_to_search() {
        let _data_dir = ScopedTestDataDir::new("import-runtime-visible");
        let source_path = unique_temp_db_path("runtime-import-source");
        let source = create_source_db(&source_path);
        let project = "/tmp/remem-import-runtime";
        source
            .execute(
                "INSERT INTO memories
                 (id, session_id, project, topic_key, title, content, memory_type, files,
                  created_at_epoch, updated_at_epoch, status, branch, scope)
                 VALUES (1, 's1', ?1, 'import-runtime-topic',
                         'Imported runtime memory',
                         'Imported memory should be visible from runtime search',
                         'decision', NULL, 100, 200, 'active', NULL, 'project')",
                [project],
            )
            .unwrap();
        drop(source);

        let runtime_conn = crate::db::open_db().expect("runtime db should open");
        let stats = import_memories_into_runtime(&source_path, &runtime_conn).unwrap();
        assert_eq!(stats.memories_imported, 1);

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
        )
        .unwrap();

        assert_eq!(results.memories.len(), 1);
        assert_eq!(results.memories[0].title, "Imported runtime memory");
        assert!(
            !crate::db::schema::default_path().exists(),
            "CLI backup import must not strand rows in schema.sqlite"
        );

        cleanup_temp_db_files(&source_path);
    }
}
