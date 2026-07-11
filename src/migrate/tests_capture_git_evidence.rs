use anyhow::Result;
use rusqlite::{params, Connection};

use super::MIGRATIONS;

#[test]
fn capture_git_evidence_migration_preserves_legacy_links() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;
    for migration in &MIGRATIONS[..66] {
        conn.execute_batch(migration.sql)?;
    }
    conn.execute(
        "INSERT INTO git_commits
         (project, repo_path, sha, short_sha, branch, message, authored_at_epoch,
          changed_files, created_at_epoch, updated_at_epoch)
         VALUES ('proj', '/repo', ?1, 'abcdef1', 'main', 'legacy link', 10,
                 '[]', 10, 10)",
        ["abcdef1234567890abcdef1234567890abcdef12"],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO hosts(name, enabled, created_at_epoch) VALUES ('codex-cli', 1, 10)",
        [],
    )?;
    let host_id: i64 =
        conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
            row.get(0)
        })?;
    conn.execute(
        "INSERT OR IGNORE INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 10, 10)",
        [],
    )?;
    let workspace_id: i64 = conn.query_row(
        "SELECT id FROM workspaces WHERE root_path = '/repo'",
        [],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, '/repo', 'repo-key', 10, 10)",
        [workspace_id],
    )?;
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, 'mapped-session', 10, 'active')",
        params![host_id, workspace_id, project_id],
    )?;
    let mapped_session_row_id = conn.last_insert_rowid();
    let mapped_sha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    conn.execute(
        "INSERT INTO git_commits
         (project, repo_path, sha, short_sha, branch, message, authored_at_epoch,
          changed_files, created_at_epoch, updated_at_epoch)
         VALUES ('/repo', '/repo', ?1, 'bbbbbbb', 'main', 'mapped link', 10,
                 '[]', 10, 10)",
        [mapped_sha],
    )?;
    let mapped_commit_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, 'mapped-session', ?2, 'git_metadata', 10)",
        params![
            mapped_commit_id,
            format!("capture-rollup-{mapped_session_row_id}")
        ],
    )?;
    let commit_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, 'legacy-session', 'legacy-memory', 'git_metadata', 10)",
        [commit_id],
    )?;

    let migration = &MIGRATIONS[66];
    assert_eq!(migration.version, 67);
    conn.execute_batch(migration.sql)?;

    let legacy: (i64, Option<i64>, String, String, String) = conn.query_row(
        "SELECT commit_id, session_row_id, session_id, memory_session_id, source
         FROM git_commit_sessions WHERE commit_id = ?1",
        [commit_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    assert_eq!(legacy.0, commit_id);
    assert_eq!(legacy.1, None);
    assert_eq!(legacy.2, "legacy-session");
    assert_eq!(legacy.3, "legacy-memory");
    assert_eq!(legacy.4, "git_metadata");
    let mapped: Option<i64> = conn.query_row(
        "SELECT session_row_id FROM git_commit_sessions WHERE commit_id = ?1",
        [mapped_commit_id],
        |row| row.get(0),
    )?;
    assert_eq!(mapped, Some(mapped_session_row_id));
    conn.prepare("SELECT event_row_id, sha, metadata_json, evidence_kind, evidence_locator FROM captured_event_commits LIMIT 0")?;
    let metadata = crate::git_util::GitCommitMetadata {
        repo_path: "/repo".to_string(),
        sha: mapped_sha.to_string(),
        short_sha: "bbbbbbb".to_string(),
        branch: Some("main".to_string()),
        message: Some("mapped link".to_string()),
        authored_at_epoch: Some(10),
        changed_files: Vec::new(),
    };
    crate::git_trace::link_captured_git_metadata_to_session(
        &conn,
        "/repo",
        mapped_session_row_id,
        "mapped-session",
        &format!("capture-rollup-{mapped_session_row_id}"),
        &metadata,
    )?;
    let mapped_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM git_commit_sessions WHERE commit_id = ?1",
        [mapped_commit_id],
        |row| row.get(0),
    )?;
    assert_eq!(mapped_count, 1);
    let delete_error = conn
        .execute(
            "DELETE FROM sessions WHERE id = ?1",
            [mapped_session_row_id],
        )
        .expect_err("durable commit link should restrict session deletion");
    assert!(delete_error.to_string().contains("FOREIGN KEY"));
    let foreign_key_errors: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_foreign_key_check",
        params![],
        |row| row.get(0),
    )?;
    assert_eq!(foreign_key_errors, 0);
    Ok(())
}
