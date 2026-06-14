use anyhow::Result;
use rusqlite::{params, Connection};

use super::*;
use crate::memory::Memory;

fn staleness_memory(updated_at_epoch: i64, status: &str) -> Memory {
    Memory {
        id: 1,
        session_id: None,
        project: "/repo".to_string(),
        topic_key: None,
        title: "Staleness fixture".to_string(),
        text: "body".to_string(),
        memory_type: "decision".to_string(),
        files: None,
        created_at_epoch: updated_at_epoch,
        updated_at_epoch,
        status: status.to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

#[test]
fn labels_memory_status_age_and_untracked_source_anchor() {
    let label = memory_staleness_label(&staleness_memory(1_700_000_000, "active"), 1_700_000_000);

    assert_eq!(label.status, "active");
    assert_eq!(label.age, "fresh");
    assert_eq!(label.source_anchor, "untracked");
    assert_eq!(
        label.label,
        "status=active; staleness=fresh; source_anchor=untracked"
    );
    assert_eq!(
        memory_staleness(&staleness_memory(1_700_000_000, "active"), 1_700_000_000),
        "status=active; staleness=fresh; source_anchor=untracked"
    );
}

#[test]
fn classifies_age_buckets() {
    let now = 1_700_000_000;

    assert_eq!(age_staleness(now - 30 * 86_400, now), "fresh");
    assert_eq!(age_staleness(now - 31 * 86_400, now), "aging");
    assert_eq!(age_staleness(now - 91 * 86_400, now), "old");
}

#[test]
fn source_anchor_marks_untracked_without_files_or_commit_link() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let mut memory = staleness_memory(1_700_000_000, "active");
    memory.session_id = Some("mem-session-1".to_string());

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "untracked");
    Ok(())
}

#[test]
fn source_anchor_tracks_commit_without_later_file_change() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    link_staleness_commit(
        &conn,
        1,
        "source-sha",
        100,
        &["src/lib.rs"],
        "mem-session-1",
    )?;
    insert_staleness_commit(&conn, 2, "later-sha", 200, &["README.md"])?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "tracked");
    assert!(label.label.contains("source_anchor=tracked"));
    Ok(())
}

#[test]
fn source_anchor_requires_verification_after_later_file_change() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    link_staleness_commit(
        &conn,
        1,
        "source-sha",
        100,
        &["src/lib.rs"],
        "mem-session-1",
    )?;
    insert_staleness_commit(&conn, 2, "later-sha", 200, &["src/lib.rs"])?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    assert!(label.label.contains("source_anchor=verify-before-trust"));
    Ok(())
}

#[test]
fn source_anchor_matches_directory_overlap() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let memory = tracked_staleness_memory(Some(r#"["src/context"]"#));
    link_staleness_commit(
        &conn,
        1,
        "source-sha",
        100,
        &["src/context/query.rs"],
        "mem-session-1",
    )?;
    insert_staleness_commit(&conn, 2, "later-sha", 200, &["src/context/render.rs"])?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

#[test]
fn source_anchor_uses_candidate_evidence_without_memory_session_or_files() -> Result<()> {
    let conn = migrated_staleness_db()?;
    seed_project_session(&conn, "auto-session")?;
    seed_candidate_evidence_memory(&conn, 42, 20, 10, "auto-session", "src/auto.rs")?;
    link_staleness_commit(
        &conn,
        1,
        "source-auto",
        100,
        &["src/auto.rs"],
        "auto-session",
    )?;
    insert_staleness_commit(&conn, 2, "later-auto", 200, &["src/auto.rs"])?;

    let mut memory = staleness_memory(1_700_000_000, "active");
    memory.id = 42;
    memory.project = "proj".to_string();

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

#[test]
fn source_anchor_limits_legacy_files_to_cited_evidence_events() -> Result<()> {
    let conn = migrated_staleness_db()?;
    seed_project_session(&conn, "auto-session")?;
    seed_candidate_evidence_memory(&conn, 42, 20, 10, "auto-session", "src/cited.rs")?;
    insert_captured_event(&conn, 11, "auto-session")?;
    let unrelated_files = serde_json::to_string(&vec!["src/unrelated.rs"])?;
    conn.execute(
        "INSERT INTO events
         (id, session_id, project, event_type, summary, files, created_at_epoch)
         VALUES (11, 'auto-session', 'proj', 'file_edit', 'unrelated edit', ?1, 110)",
        [unrelated_files],
    )?;
    link_staleness_commit(
        &conn,
        1,
        "source-cited",
        100,
        &["src/cited.rs"],
        "auto-session",
    )?;
    insert_staleness_commit(&conn, 2, "later-unrelated", 200, &["src/unrelated.rs"])?;

    let mut memory = staleness_memory(1_700_000_000, "active");
    memory.id = 42;
    memory.project = "proj".to_string();

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "tracked");
    Ok(())
}

#[test]
fn source_anchor_uses_observation_evidence_files_by_capture_session_row() -> Result<()> {
    let conn = migrated_staleness_db()?;
    seed_project_session(&conn, "auto-session")?;
    insert_captured_event(&conn, 10, "auto-session")?;
    seed_candidate_memory_without_legacy_event(&conn, 42, 20, 10)?;
    let evidence_json = serde_json::to_string(&vec![10])?;
    let files_json = serde_json::to_string(&vec!["src/observed.rs"])?;
    conn.execute(
        "INSERT INTO observations
         (memory_session_id, project, type, title, files_modified, created_at_epoch,
          session_row_id, evidence_event_ids)
         VALUES ('capture-observation-9001', 'proj', 'discovery', 'Observed files',
                 ?1, 100, 9001, ?2)",
        params![files_json, evidence_json],
    )?;
    link_staleness_commit_with_sessions(
        &conn,
        1,
        "source-observed",
        100,
        &["src/observed.rs"],
        "auto-session",
        "capture-observation-9001",
    )?;
    insert_staleness_commit(&conn, 2, "later-observed", 200, &["src/observed.rs"])?;

    let mut memory = staleness_memory(1_700_000_000, "active");
    memory.id = 42;
    memory.project = "proj".to_string();

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

#[test]
fn source_anchor_ignores_session_commits_created_after_memory() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let mut memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    memory.created_at_epoch = 150;
    memory.updated_at_epoch = 150;
    link_staleness_commit(
        &conn,
        1,
        "source-before-memory",
        100,
        &["src/lib.rs"],
        "mem-session-1",
    )?;
    link_staleness_commit(
        &conn,
        2,
        "future-same-session",
        200,
        &["src/lib.rs"],
        "mem-session-1",
    )?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

#[test]
fn source_anchor_requires_source_commit_file_overlap() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let mut memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    memory.created_at_epoch = 150;
    memory.updated_at_epoch = 150;
    link_staleness_commit(
        &conn,
        1,
        "source-unrelated",
        100,
        &["README.md"],
        "mem-session-1",
    )?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "untracked");
    Ok(())
}

#[test]
fn source_anchor_ignores_later_commit_on_unrelated_branch() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let mut memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    memory.branch = Some("main".to_string());
    link_staleness_commit_on_branch(
        &conn,
        1,
        "source-main",
        100,
        &["src/lib.rs"],
        "mem-session-1",
        "main",
    )?;
    insert_staleness_commit_on_branch(&conn, 2, "later-feature", 200, &["src/lib.rs"], "feature")?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "tracked");
    Ok(())
}

#[test]
fn source_anchor_requires_verification_after_same_branch_file_change() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let mut memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    memory.branch = Some("main".to_string());
    link_staleness_commit_on_branch(
        &conn,
        1,
        "source-main",
        100,
        &["src/lib.rs"],
        "mem-session-1",
        "main",
    )?;
    insert_staleness_commit_on_branch(&conn, 2, "later-main", 200, &["src/lib.rs"], "main")?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

#[test]
fn source_anchor_treats_branchless_source_commit_as_branch_neutral() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let mut memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    memory.branch = Some("main".to_string());
    link_staleness_commit_without_branch(
        &conn,
        1,
        "source-branchless",
        100,
        &["src/lib.rs"],
        "mem-session-1",
    )?;
    insert_staleness_commit_on_branch(&conn, 2, "later-main", 200, &["src/lib.rs"], "main")?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

#[test]
fn source_anchor_treats_branchless_later_commit_as_branch_neutral() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let mut memory = tracked_staleness_memory(Some(r#"["src/lib.rs"]"#));
    memory.branch = Some("main".to_string());
    link_staleness_commit_on_branch(
        &conn,
        1,
        "source-main",
        100,
        &["src/lib.rs"],
        "mem-session-1",
        "main",
    )?;
    insert_staleness_commit_without_branch(&conn, 2, "later-branchless", 200, &["src/lib.rs"])?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

#[test]
fn source_anchor_normalizes_absolute_memory_files_against_project() -> Result<()> {
    let conn = migrated_staleness_db()?;
    let memory = tracked_staleness_memory(Some(r#"["/proj/src/lib.rs"]"#));
    link_staleness_commit(
        &conn,
        1,
        "source-relative",
        100,
        &["src/lib.rs"],
        "mem-session-1",
    )?;
    insert_staleness_commit(&conn, 2, "later-relative", 200, &["src/lib.rs"])?;

    let label = memory_staleness_label_with_conn(&conn, &memory, 1_700_000_000)?;

    assert_eq!(label.source_anchor, "verify-before-trust");
    Ok(())
}

fn migrated_staleness_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn tracked_staleness_memory(files: Option<&str>) -> Memory {
    let mut memory = staleness_memory(1_700_000_000, "active");
    memory.session_id = Some("mem-session-1".to_string());
    memory.project = "proj".to_string();
    memory.files = files.map(str::to_string);
    memory
}

fn seed_project_session(conn: &Connection, session_id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO hosts (id, name, created_at_epoch) VALUES (9001, 'staleness-test-host', 0)",
        [],
    )?;
    conn.execute(
        "INSERT INTO workspaces (id, root_path, created_at_epoch, updated_at_epoch)
         VALUES (9001, '/repo/staleness-test', 0, 0)",
        [],
    )?;
    conn.execute(
        "INSERT INTO projects
         (id, workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (9001, 9001, 'proj', 'proj', 0, 0)",
        [],
    )?;
    conn.execute(
        "INSERT INTO sessions
         (id, host_id, workspace_id, project_id, session_id, last_seen_at_epoch, status)
         VALUES (9001, 9001, 9001, 9001, ?1, 0, 'active')",
        [session_id],
    )?;
    Ok(())
}

fn seed_candidate_evidence_memory(
    conn: &Connection,
    memory_id: i64,
    candidate_id: i64,
    event_id: i64,
    session_id: &str,
    file: &str,
) -> Result<()> {
    let files_json = serde_json::to_string(&vec![file])?;
    insert_captured_event(conn, event_id, session_id)?;
    conn.execute(
        "INSERT INTO events
         (id, session_id, project, event_type, summary, files, created_at_epoch)
         VALUES (?1, ?2, 'proj', 'file_edit', 'edited source', ?3, 100)",
        params![event_id, session_id, files_json],
    )?;
    seed_candidate_memory_without_legacy_event(conn, memory_id, candidate_id, event_id)?;
    Ok(())
}

fn insert_captured_event(conn: &Connection, event_id: i64, session_id: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO captured_events
         (id, host_id, workspace_id, project_id, session_row_id, session_id, event_id,
          event_type, content_hash, retention_class, created_at_epoch, inserted_at_epoch)
         VALUES (?1, 9001, 9001, 9001, 9001, ?2, ?3, 'tool', ?4,
                 'normal', 100, 100)",
        params![
            event_id,
            session_id,
            format!("event-{event_id}"),
            format!("hash-{event_id}")
        ],
    )?;
    Ok(())
}

fn seed_candidate_memory_without_legacy_event(
    conn: &Connection,
    memory_id: i64,
    candidate_id: i64,
    event_id: i64,
) -> Result<()> {
    let evidence_json = serde_json::to_string(&vec![event_id])?;
    conn.execute(
        "INSERT INTO memory_candidates
         (id, project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 9001, 'project', 'decision', 'auto-topic', 'auto memory',
                 ?2, 0.9, 'low', 'auto_promoted', 100, 100)",
        params![candidate_id, evidence_json],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope,
          evidence_event_ids, source_candidate_id, source_project)
         VALUES (?1, NULL, 'proj', 'auto-topic', 'Auto memory', 'auto memory',
                 'decision', NULL, 100, 100, 'active', NULL, 'project',
                 NULL, ?2, 'proj')",
        params![memory_id, candidate_id],
    )?;
    Ok(())
}

fn link_staleness_commit(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    memory_session_id: &str,
) -> Result<()> {
    link_staleness_commit_on_branch(
        conn,
        id,
        sha,
        epoch,
        changed_files,
        memory_session_id,
        "main",
    )
}

fn link_staleness_commit_on_branch(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    memory_session_id: &str,
    branch: &str,
) -> Result<()> {
    insert_staleness_commit_with_branch(conn, id, sha, epoch, changed_files, Some(branch))?;
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, ?2, ?3, 'test', ?4)",
        params![id, format!("content-{id}"), memory_session_id, epoch],
    )?;
    Ok(())
}

fn link_staleness_commit_without_branch(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    memory_session_id: &str,
) -> Result<()> {
    insert_staleness_commit_with_branch(conn, id, sha, epoch, changed_files, None)?;
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, ?2, ?3, 'test', ?4)",
        params![id, format!("content-{id}"), memory_session_id, epoch],
    )?;
    Ok(())
}

fn link_staleness_commit_with_sessions(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    session_id: &str,
    memory_session_id: &str,
) -> Result<()> {
    insert_staleness_commit_with_branch(conn, id, sha, epoch, changed_files, Some("main"))?;
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, ?2, ?3, 'test', ?4)",
        params![id, session_id, memory_session_id, epoch],
    )?;
    Ok(())
}

fn insert_staleness_commit(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
) -> Result<()> {
    insert_staleness_commit_on_branch(conn, id, sha, epoch, changed_files, "main")
}

fn insert_staleness_commit_on_branch(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    branch: &str,
) -> Result<()> {
    insert_staleness_commit_with_branch(conn, id, sha, epoch, changed_files, Some(branch))
}

fn insert_staleness_commit_without_branch(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
) -> Result<()> {
    insert_staleness_commit_with_branch(conn, id, sha, epoch, changed_files, None)
}

fn insert_staleness_commit_with_branch(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    branch: Option<&str>,
) -> Result<()> {
    let changed_files = serde_json::to_string(changed_files)?;
    conn.execute(
        "INSERT INTO git_commits
         (id, project, repo_path, sha, short_sha, branch, message,
          authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'proj', '/repo', ?2, ?2, ?3, NULL, ?4, ?5, ?4, ?4)",
        params![id, sha, branch, epoch, changed_files],
    )?;
    Ok(())
}
