use rusqlite::params;

use super::super::host::HostKind;
use super::super::invocation::ContextInvocation;
use super::super::render::generate_context_for_test;

fn strict_invocation(
    cwd: &str,
    transcript_path: &std::path::Path,
    session_id: &str,
) -> ContextInvocation {
    ContextInvocation {
        cwd: cwd.to_string(),
        project: crate::db::project_from_cwd(cwd),
        session_id: Some(session_id.to_string()),
        transcript_path: Some(transcript_path.to_string_lossy().to_string()),
        source: Some("session_start".to_string()),
        host: HostKind::CodexCli,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: Some("strict".to_string()),
    }
}

fn gate_row(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> anyhow::Result<(i64, Option<String>)> {
    conn.query_row(
        "SELECT suppress_count, data_version
         FROM context_injections
         WHERE session_id = ?1",
        [session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(Into::into)
}

#[test]
fn strict_context_pipeline_opens_one_database_connection() -> anyhow::Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("context-single-db-open");
    let setup = crate::db::test_support::runtime_connection()?;
    drop(setup);

    crate::db::test_support::reset_runtime_connection_open_count();
    let cwd = data_dir.path.to_string_lossy().to_string();
    let transcript_path = data_dir.path.join("transcript.jsonl");
    let invocation = strict_invocation(&cwd, &transcript_path, "sess-single-db-open");

    generate_context_for_test(invocation, true)?;

    assert_eq!(crate::db::test_support::runtime_connection_open_count(), 1);
    let conn = crate::db::test_support::runtime_connection()?;
    let (_, data_version) = gate_row(&conn, "sess-single-db-open")?;
    assert!(
        data_version
            .as_deref()
            .is_some_and(|value| !value.is_empty()),
        "first strict render should store a reusable data_version"
    );
    Ok(())
}

#[test]
fn strict_context_pipeline_does_not_retain_context_load_errors() -> anyhow::Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-load-error");
    let setup = crate::db::test_support::runtime_connection()?;
    setup.execute("DROP TABLE memory_lessons", [])?;
    drop(setup);

    let cwd = data_dir.path.to_string_lossy().to_string();
    let transcript_path = data_dir.path.join("transcript.jsonl");
    let invocation = strict_invocation(&cwd, &transcript_path, "sess-gate-load-error");

    generate_context_for_test(invocation, true)?;

    let conn = crate::db::test_support::runtime_connection()?;
    let retained_rows: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM context_injections
         WHERE session_id = 'sess-gate-load-error'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        retained_rows, 0,
        "strict gate should not retain or suppress context load error output"
    );
    Ok(())
}

#[test]
fn strict_context_pipeline_load_errors_do_not_suppress_existing_gate_row() -> anyhow::Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("context-gate-existing-load-error");
    let setup = crate::db::test_support::runtime_connection()?;
    drop(setup);

    let cwd = data_dir.path.to_string_lossy().to_string();
    let transcript_path = data_dir.path.join("transcript.jsonl");
    let invocation = strict_invocation(&cwd, &transcript_path, "sess-gate-existing-load-error");

    generate_context_for_test(invocation.clone(), true)?;

    let conn = crate::db::test_support::runtime_connection()?;
    let (first_suppress_count, first_data_version) =
        gate_row(&conn, "sess-gate-existing-load-error")?;
    assert_eq!(first_suppress_count, 0);
    assert!(first_data_version
        .as_deref()
        .is_some_and(|value| value.starts_with("v3:")));

    conn.execute("DROP TABLE memory_lessons", [])?;
    drop(conn);

    generate_context_for_test(invocation, true)?;

    let conn = crate::db::test_support::runtime_connection()?;
    let (second_suppress_count, second_data_version) =
        gate_row(&conn, "sess-gate-existing-load-error")?;
    assert_eq!(
        second_suppress_count, 0,
        "load-error render should fail open instead of suppressing the existing gate row"
    );
    assert_eq!(second_data_version, first_data_version);
    Ok(())
}

#[test]
fn strict_context_pipeline_hybrid_fts_failure_does_not_suppress_existing_gate_row(
) -> anyhow::Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-fts-load-error");
    let conn = crate::db::test_support::runtime_connection()?;
    let cwd = data_dir.path.to_string_lossy().to_string();
    let project = crate::db::project_from_cwd(&cwd);
    conn.execute(
        "INSERT INTO workstreams
         (project, title, status, next_action, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key, context_class)
         VALUES (?1, 'Hybrid substrate gate check', 'active', 'Read recent context',
                 2000, 2000, ?1, ?1, 'repo', ?1, 'startup_core')",
        params![project],
    )?;
    drop(conn);

    let transcript_path = data_dir.path.join("transcript.jsonl");
    let invocation = strict_invocation(&cwd, &transcript_path, "sess-gate-fts-load-error");
    generate_context_for_test(invocation.clone(), true)?;

    let conn = crate::db::test_support::runtime_connection()?;
    let (first_suppress_count, first_data_version) = gate_row(&conn, "sess-gate-fts-load-error")?;
    assert_eq!(first_suppress_count, 0);
    assert!(first_data_version
        .as_deref()
        .is_some_and(|value| value.starts_with("v3:")));

    conn.execute("DROP TABLE memories_fts", [])?;
    drop(conn);

    generate_context_for_test(invocation, true)?;

    let conn = crate::db::test_support::runtime_connection()?;
    let (second_suppress_count, second_data_version) = gate_row(&conn, "sess-gate-fts-load-error")?;
    assert_eq!(
        second_suppress_count, 0,
        "hybrid FTS load errors should fail open instead of pre-render suppressing"
    );
    assert_eq!(second_data_version, first_data_version);
    Ok(())
}

#[test]
fn strict_context_pipeline_suppresses_with_null_fts_search_context() -> anyhow::Result<()> {
    let data_dir =
        crate::db::test_support::ScopedTestDataDir::new("context-gate-null-search-context");
    let conn = crate::db::test_support::runtime_connection()?;
    let cwd = data_dir.path.to_string_lossy().to_string();
    let project = crate::db::project_from_cwd(&cwd);
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, owner_scope, owner_key, target_project)
         VALUES (1, 'sess-source', ?1, 'Null search context', 'Legacy row without search_context.',
                 'decision', 1, 1, 'active', 'repo', ?1, ?1)",
        params![project],
    )?;
    let stored_search_context: Option<String> = conn.query_row(
        "SELECT search_context FROM memories WHERE id = 1",
        [],
        |row| row.get(0),
    )?;
    assert!(stored_search_context.is_none());
    drop(conn);

    let transcript_path = data_dir.path.join("transcript.jsonl");
    let invocation = strict_invocation(&cwd, &transcript_path, "sess-null-search-context");
    generate_context_for_test(invocation.clone(), true)?;

    let conn = crate::db::test_support::runtime_connection()?;
    let (first_suppress_count, first_data_version) = gate_row(&conn, "sess-null-search-context")?;
    assert_eq!(first_suppress_count, 0);
    assert!(first_data_version
        .as_deref()
        .is_some_and(|value| value.starts_with("v3:")));
    drop(conn);

    generate_context_for_test(invocation, true)?;

    let conn = crate::db::test_support::runtime_connection()?;
    let (second_suppress_count, second_data_version) = gate_row(&conn, "sess-null-search-context")?;
    assert_eq!(
        second_suppress_count, 1,
        "strict pre-render should suppress using the stored data_version instead of rendering again"
    );
    assert_eq!(second_data_version, first_data_version);
    Ok(())
}
