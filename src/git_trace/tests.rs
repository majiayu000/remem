use super::*;
use rusqlite::Connection;

fn migrated_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn metadata_input<'a>(changed_files: &'a [String]) -> CommitMetadataInput<'a> {
    CommitMetadataInput {
        project: "proj",
        repo_path: Some("/repo"),
        sha: "abcdef1234567890abcdef1234567890abcdef12",
        short_sha: Some("abcdef1"),
        branch: Some("main"),
        message: Some("Add traceability"),
        authored_at_epoch: Some(1_700_000_000),
        changed_files,
    }
}

#[test]
fn link_lookup_and_session_reverse_lookup_are_idempotent() -> Result<()> {
    let conn = migrated_db()?;
    let changed_files = vec!["src/lib.rs".to_string(), "README.md".to_string()];
    let input = CommitLinkInput {
        metadata: metadata_input(&changed_files),
        session_id: "content-session-1",
        memory_session_id: Some("mem-session-1"),
        source: "git_metadata",
    };

    let first_id = link_commit_to_session(&conn, &input)?;
    let second_id = link_commit_to_session(&conn, &input)?;
    assert_eq!(first_id, second_id);

    let link_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM git_commit_sessions WHERE commit_id = ?1",
        [first_id],
        |row| row.get(0),
    )?;
    assert_eq!(link_count, 1);

    let full = lookup_commit(
        &conn,
        Some("proj"),
        "abcdef1234567890abcdef1234567890abcdef12",
    )?;
    assert_eq!(full.len(), 1);
    assert_eq!(full[0].git.short_sha, "abcdef1");
    assert_eq!(full[0].git.changed_files, changed_files);
    assert_eq!(full[0].sessions.len(), 1);
    assert_eq!(full[0].sessions[0].session_id, "content-session-1");
    let public_json = serde_json::to_value(&full[0])?;
    assert!(public_json["sessions"][0].get("session_row_id").is_none());

    let short = lookup_commit(&conn, Some("proj"), "abcdef1")?;
    assert_eq!(short.len(), 1);
    assert_eq!(short[0].git.sha, full[0].git.sha);

    let session_commits = commits_for_session(&conn, Some("proj"), "content-session-1", 10)?;
    assert_eq!(session_commits.len(), 1);
    assert_eq!(session_commits[0].git.short_sha, "abcdef1");

    let memory_session_commits = commits_for_session(&conn, Some("proj"), "mem-session-1", 10)?;
    assert_eq!(memory_session_commits.len(), 1);
    assert_eq!(
        memory_session_commits[0].link.memory_session_id.as_deref(),
        Some("mem-session-1")
    );

    link_observed_commit_to_session(
        &conn,
        "proj",
        "content-session-2",
        "mem-session-2",
        "abcdef1",
        Some("main"),
        None,
    )?;
    let commit_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commits", [], |row| row.get(0))?;
    assert_eq!(commit_count, 1);
    let relinked = lookup_commit(&conn, Some("proj"), "abcdef1")?;
    assert_eq!(relinked[0].sessions.len(), 2);
    Ok(())
}

#[test]
fn full_metadata_upgrades_existing_short_sha_record() -> Result<()> {
    let conn = migrated_db()?;
    link_observed_commit_to_session(
        &conn,
        "proj",
        "content-session-1",
        "mem-session-1",
        "abcdef1",
        Some("main"),
        None,
    )?;

    let changed_files = vec!["src/git_trace.rs".to_string()];
    link_commit_to_session(
        &conn,
        &CommitLinkInput {
            metadata: metadata_input(&changed_files),
            session_id: "content-session-2",
            memory_session_id: Some("mem-session-2"),
            source: "git_metadata",
        },
    )?;

    let commit_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commits", [], |row| row.get(0))?;
    assert_eq!(commit_count, 1);
    let results = lookup_commit(
        &conn,
        Some("proj"),
        "abcdef1234567890abcdef1234567890abcdef12",
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].git.short_sha, "abcdef1");
    assert_eq!(results[0].git.changed_files, changed_files);
    assert_eq!(results[0].sessions.len(), 2);
    Ok(())
}

#[test]
fn full_metadata_upgrades_existing_long_abbrev_record() -> Result<()> {
    let conn = migrated_db()?;
    link_observed_commit_to_session(
        &conn,
        "proj",
        "content-session-1",
        "mem-session-1",
        "abcdef123456",
        Some("main"),
        None,
    )?;

    let changed_files = vec!["src/git_trace.rs".to_string()];
    link_commit_to_session(
        &conn,
        &CommitLinkInput {
            metadata: metadata_input(&changed_files),
            session_id: "content-session-2",
            memory_session_id: Some("mem-session-2"),
            source: "git_metadata",
        },
    )?;

    let commit_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM git_commits", [], |row| row.get(0))?;
    assert_eq!(commit_count, 1);
    let results = lookup_commit(
        &conn,
        Some("proj"),
        "abcdef1234567890abcdef1234567890abcdef12",
    )?;
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].git.sha,
        "abcdef1234567890abcdef1234567890abcdef12"
    );
    assert_eq!(results[0].git.short_sha, "abcdef1");
    assert_eq!(results[0].sessions.len(), 2);
    Ok(())
}

#[test]
fn relink_preserves_git_metadata_source() -> Result<()> {
    let conn = migrated_db()?;
    let changed_files = vec!["src/git_trace.rs".to_string()];
    link_observed_commit_to_session(
        &conn,
        "proj",
        "content-session-1",
        "mem-session-1",
        "abcdef1",
        Some("main"),
        None,
    )?;
    link_commit_to_session(
        &conn,
        &CommitLinkInput {
            metadata: metadata_input(&changed_files),
            session_id: "content-session-1",
            memory_session_id: Some("mem-session-1"),
            source: "git_metadata",
        },
    )?;
    link_observed_commit_to_session(
        &conn,
        "proj",
        "content-session-1",
        "mem-session-1",
        "abcdef1",
        Some("main"),
        None,
    )?;

    let source: String = conn.query_row(
        "SELECT source
         FROM git_commit_sessions
         WHERE session_id = 'content-session-1'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(source, "git_metadata");
    Ok(())
}

#[test]
fn commit_metadata_without_link_returns_no_sessions() -> Result<()> {
    let conn = migrated_db()?;
    let changed_files = Vec::new();
    let commit_id = upsert_commit_metadata(&conn, &metadata_input(&changed_files))?;
    assert!(commit_id > 0);

    let results = lookup_commit(&conn, Some("proj"), "abcdef1")?;
    assert_eq!(results.len(), 1);
    assert!(results[0].sessions.is_empty());
    Ok(())
}

#[test]
fn summary_is_memory_derived_and_optional() -> Result<()> {
    let conn = migrated_db()?;
    let changed_files = Vec::new();
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at, created_at_epoch)
         VALUES ('mem-session-1', 'proj', 'Need traceability', 'Linked commits',
                 '2026-01-01T00:00:00Z', 1)",
        [],
    )?;
    link_commit_to_session(
        &conn,
        &CommitLinkInput {
            metadata: metadata_input(&changed_files),
            session_id: "content-session-1",
            memory_session_id: Some("mem-session-1"),
            source: "git_metadata",
        },
    )?;

    let results = lookup_commit(&conn, Some("proj"), "abcdef1")?;
    let summary = results[0].sessions[0]
        .summary
        .as_ref()
        .expect("linked summary should be returned");
    assert_eq!(summary.request.as_deref(), Some("Need traceability"));
    assert_eq!(summary.completed.as_deref(), Some("Linked commits"));
    Ok(())
}
