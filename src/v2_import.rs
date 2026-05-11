//! v1 -> v2 best-effort data migration. Per
//! SPEC-memory-system-v2.1-revisions §4 D5, transcripts are NOT replayed by
//! default. Only the v1 `memories` table is migrated; observations /
//! pending_observations / events / raw messages are intentionally left out
//! because they have no clean v2 mapping at the granularity of
//! `evidence_event_ids` (the v2 provenance contract).

use anyhow::{anyhow, Context, Result};
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImportStats {
    pub memories_imported: usize,
    pub memories_skipped: usize,
    pub workspaces_created: usize,
    pub projects_created: usize,
}

/// Read v1 memories from `v1_path` and INSERT them into the v2 DB connected
/// as `v2_conn`. Best-effort: rows that violate v2 constraints (e.g. the
/// unique `(scope, project_id, topic_key)` index) are counted as skipped,
/// not propagated as an error. Confidence defaults to 0.7 because v1 had no
/// calibration; `evidence_event_ids` is `'[]'` since v1 did not preserve
/// event-level provenance.
pub fn import_legacy_memories(v1_path: &Path, v2_conn: &Connection) -> Result<ImportStats> {
    if !v1_path.exists() {
        return Err(anyhow!("source v1 db not found at {}", v1_path.display()));
    }
    let mut v1 = Connection::open_with_flags(v1_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open v1 source {}", v1_path.display()))?;
    let has_cipher_key = crate::db::apply_cipher_key_if_available(&v1)
        .with_context(|| format!("unlock v1 source {}", v1_path.display()))?;
    if has_cipher_key && !crate::db::can_read_schema(&v1) {
        drop(v1);
        v1 = Connection::open_with_flags(v1_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("reopen unencrypted v1 source {}", v1_path.display()))?;
    }

    let mut stmt = v1.prepare(
        "SELECT id, project, topic_key, title, content, memory_type, scope, status,
                created_at_epoch, updated_at_epoch
         FROM memories",
    )?;
    let mut rows = stmt.query([])?;

    let mut stats = ImportStats::default();
    while let Some(row) = rows.next()? {
        let v1_id: i64 = row.get(0)?;
        let project: String = row.get(1)?;
        let topic_key: Option<String> = row.get(2).ok();
        let title: String = row.get(3)?;
        let content: String = row.get(4)?;
        let memory_type: String = row.get(5)?;
        let scope: String = row
            .get::<_, Option<String>>(6)?
            .unwrap_or_else(|| "project".to_string());
        let status: String = row
            .get::<_, Option<String>>(7)?
            .unwrap_or_else(|| "active".to_string());
        let created_at: i64 = row.get(8)?;
        let updated_at: i64 = row.get(9)?;

        let project_id = if scope == "global" {
            None
        } else {
            let (id, ws_inserted, p_inserted) = find_or_create_project(v2_conn, &project)?;
            if ws_inserted {
                stats.workspaces_created += 1;
            }
            if p_inserted {
                stats.projects_created += 1;
            }
            Some(id)
        };

        let topic_key = topic_key.unwrap_or_else(|| format!("legacy-{v1_id}"));
        let text = match (title.is_empty(), content.is_empty()) {
            (true, true) => continue,
            (true, _) => content,
            (_, true) => title,
            _ => format!("{title}\n\n{content}"),
        };

        let result = v2_conn.execute(
            "INSERT INTO memories(project_id, scope, memory_type, topic_key, text,
                evidence_event_ids, confidence, status, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, '[]', 0.7, ?6, ?7, ?8)",
            rusqlite::params![
                project_id,
                scope,
                memory_type,
                topic_key,
                text,
                status,
                created_at,
                updated_at
            ],
        );
        match result {
            Ok(_) => stats.memories_imported += 1,
            Err(e) => {
                crate::log::warn("import", &format!("skipped v1 memory id={v1_id}: {e}"));
                stats.memories_skipped += 1;
            }
        }
    }
    Ok(stats)
}

/// Returns `(project_id, workspace_inserted, project_inserted)`. Looks up
/// the joined `workspaces`/`projects` row first; on miss, creates the
/// workspace (if needed) and the project.
fn find_or_create_project(conn: &Connection, project_path: &str) -> Result<(i64, bool, bool)> {
    if let Ok(id) = conn.query_row(
        "SELECT p.id FROM projects p
         JOIN workspaces w ON w.id = p.workspace_id
         WHERE w.root_path = ?1 AND p.project_path = ?1",
        [project_path],
        |row| row.get::<_, i64>(0),
    ) {
        return Ok((id, false, false));
    }
    let now = chrono::Utc::now().timestamp();
    let (ws_id, ws_inserted) = match conn.query_row(
        "SELECT id FROM workspaces WHERE root_path = ?1",
        [project_path],
        |row| row.get::<_, i64>(0),
    ) {
        Ok(id) => (id, false),
        Err(_) => {
            conn.execute(
                "INSERT INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
                 VALUES (?1, ?2, ?2)",
                rusqlite::params![project_path, now],
            )?;
            (conn.last_insert_rowid(), true)
        }
    };
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key,
            created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        rusqlite::params![ws_id, project_path, project_path, now],
    )?;
    Ok((conn.last_insert_rowid(), ws_inserted, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files as cleanup, unique_temp_db_path};
    use crate::v2_db::open_v2_db_at;
    use std::path::PathBuf;

    fn unique_temp_path(label: &str) -> PathBuf {
        unique_temp_db_path(&format!("import-{label}"))
    }

    fn create_v1_db(path: &Path) -> Connection {
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

    fn create_encrypted_v1_db(path: &Path, key: &str) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "key", key).unwrap();
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
    fn missing_source_returns_error() {
        let v1 = unique_temp_path("missing");
        let v2 = unique_temp_path("v2-empty");
        let v2_conn = open_v2_db_at(&v2).unwrap();
        let err = import_legacy_memories(&v1, &v2_conn)
            .unwrap_err()
            .to_string();
        assert!(err.contains("not found"), "got: {err}");
        cleanup(&v2);
    }

    #[test]
    fn project_scope_creates_workspace_and_project() {
        let v1_path = unique_temp_path("v1-proj");
        let v2_path = unique_temp_path("v2-proj");
        {
            let v1 = create_v1_db(&v1_path);
            v1.execute(
                "INSERT INTO memories(project, topic_key, title, content, memory_type,
                  status, scope, created_at_epoch, updated_at_epoch)
                 VALUES ('/repo/foo', 'topic1', 'title', 'content', 'discovery',
                  'active', 'project', 100, 200)",
                [],
            )
            .unwrap();
        }
        let v2_conn = open_v2_db_at(&v2_path).unwrap();
        let stats = import_legacy_memories(&v1_path, &v2_conn).unwrap();
        assert_eq!(stats.memories_imported, 1);
        assert_eq!(stats.workspaces_created, 1);
        assert_eq!(stats.projects_created, 1);
        let mem_count: i64 = v2_conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mem_count, 1);
        cleanup(&v1_path);
        cleanup(&v2_path);
    }

    #[test]
    fn imports_encrypted_v1_with_cipher_key() {
        let test_dir = crate::db::test_support::ScopedTestDataDir::new("v2-import-encrypted");
        std::fs::create_dir_all(&test_dir.path).unwrap();
        let key = "import-test-key";
        std::fs::write(test_dir.path.join(".key"), key).unwrap();
        let v1_path = test_dir.path.join("legacy.sqlite");
        let v2_path = test_dir.path.join("v2.sqlite");
        {
            let v1 = create_encrypted_v1_db(&v1_path, key);
            v1.execute(
                "INSERT INTO memories(project, topic_key, title, content, memory_type,
                  status, scope, created_at_epoch, updated_at_epoch)
                 VALUES ('/repo/encrypted', 'encrypted-topic', 'encrypted title',
                  'encrypted content', 'discovery', 'active', 'project', 100, 200)",
                [],
            )
            .unwrap();
        }

        let v2_conn = open_v2_db_at(&v2_path).unwrap();
        let stats = import_legacy_memories(&v1_path, &v2_conn).unwrap();

        assert_eq!(stats.memories_imported, 1);
        let text: String = v2_conn
            .query_row("SELECT text FROM memories", [], |r| r.get(0))
            .unwrap();
        assert!(text.contains("encrypted title"), "got: {text}");
        assert!(text.contains("encrypted content"), "got: {text}");
    }

    #[test]
    fn global_scope_has_null_project_id() {
        let v1_path = unique_temp_path("v1-glob");
        let v2_path = unique_temp_path("v2-glob");
        {
            let v1 = create_v1_db(&v1_path);
            v1.execute(
                "INSERT INTO memories(project, topic_key, title, content, memory_type,
                  status, scope, created_at_epoch, updated_at_epoch)
                 VALUES ('/anywhere', 'g1', 'gtitle', 'gcontent', 'preference',
                  'active', 'global', 100, 200)",
                [],
            )
            .unwrap();
        }
        let v2_conn = open_v2_db_at(&v2_path).unwrap();
        let stats = import_legacy_memories(&v1_path, &v2_conn).unwrap();
        assert_eq!(stats.memories_imported, 1);
        assert_eq!(stats.workspaces_created, 0, "global scope skips workspace");
        let project_id: Option<i64> = v2_conn
            .query_row("SELECT project_id FROM memories", [], |r| r.get(0))
            .unwrap();
        assert_eq!(project_id, None);
        cleanup(&v1_path);
        cleanup(&v2_path);
    }

    #[test]
    fn same_project_reuses_workspace() {
        let v1_path = unique_temp_path("v1-reuse");
        let v2_path = unique_temp_path("v2-reuse");
        {
            let v1 = create_v1_db(&v1_path);
            v1.execute(
                "INSERT INTO memories(project, topic_key, title, content, memory_type,
                  status, scope, created_at_epoch, updated_at_epoch)
                 VALUES ('/repo/a', 't1', 'title1', 'c1', 'discovery', 'active', 'project', 100, 200),
                        ('/repo/a', 't2', 'title2', 'c2', 'bugfix', 'active', 'project', 300, 400)",
                [],
            )
            .unwrap();
        }
        let v2_conn = open_v2_db_at(&v2_path).unwrap();
        let stats = import_legacy_memories(&v1_path, &v2_conn).unwrap();
        assert_eq!(stats.memories_imported, 2);
        assert_eq!(stats.workspaces_created, 1);
        assert_eq!(stats.projects_created, 1);
        cleanup(&v1_path);
        cleanup(&v2_path);
    }

    #[test]
    fn duplicate_topic_key_is_skipped_not_failed() {
        let v1_path = unique_temp_path("v1-dup");
        let v2_path = unique_temp_path("v2-dup");
        {
            let v1 = create_v1_db(&v1_path);
            v1.execute(
                "INSERT INTO memories(project, topic_key, title, content, memory_type,
                  status, scope, created_at_epoch, updated_at_epoch)
                 VALUES ('/repo/x', 'same-topic', 'title1', 'c1', 'discovery', 'active', 'project', 100, 200),
                        ('/repo/x', 'same-topic', 'title2', 'c2', 'bugfix', 'active', 'project', 300, 400)",
                [],
            )
            .unwrap();
        }
        let v2_conn = open_v2_db_at(&v2_path).unwrap();
        let stats = import_legacy_memories(&v1_path, &v2_conn).unwrap();
        assert_eq!(stats.memories_imported, 1);
        assert_eq!(
            stats.memories_skipped, 1,
            "second row violates UNIQUE topic"
        );
        cleanup(&v1_path);
        cleanup(&v2_path);
    }

    #[test]
    fn null_topic_key_is_synthesized() {
        let v1_path = unique_temp_path("v1-nullkey");
        let v2_path = unique_temp_path("v2-nullkey");
        {
            let v1 = create_v1_db(&v1_path);
            v1.execute(
                "INSERT INTO memories(project, title, content, memory_type, status, scope,
                  created_at_epoch, updated_at_epoch)
                 VALUES ('/repo/n', 'title', 'content', 'discovery', 'active', 'project', 100, 200)",
                [],
            )
            .unwrap();
        }
        let v2_conn = open_v2_db_at(&v2_path).unwrap();
        let stats = import_legacy_memories(&v1_path, &v2_conn).unwrap();
        assert_eq!(stats.memories_imported, 1);
        let topic: String = v2_conn
            .query_row("SELECT topic_key FROM memories", [], |r| r.get(0))
            .unwrap();
        assert!(topic.starts_with("legacy-"), "got: {topic}");
        cleanup(&v1_path);
        cleanup(&v2_path);
    }
}
