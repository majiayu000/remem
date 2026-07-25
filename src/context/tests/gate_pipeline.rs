use rusqlite::{params, Connection};

use super::super::host::HostKind;
use super::super::invocation::ContextInvocation;
use super::super::render::{generate_context_for_test, generate_context_output_for_test};

struct ScopedClaudeHome {
    previous_home: Option<std::ffi::OsString>,
    path: std::path::PathBuf,
}

impl ScopedClaudeHome {
    fn with_incomplete_hooks(label: &str) -> anyhow::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "remem-claude-home-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = std::fs::remove_dir_all(&path);
        let claude_dir = path.join(".claude");
        std::fs::create_dir_all(&claude_dir)?;
        std::fs::write(
            claude_dir.join("settings.json"),
            r#"{"hooks":{"SessionStart":[{"matcher":"startup|resume|clear|compact","hooks":[{"command":"/tmp/remem context --host claude-code","timeout":15}]}],"UserPromptSubmit":[{"hooks":[{"command":"/tmp/remem session-init --host claude-code","timeout":15}]}],"PreCompact":[{"hooks":[{"command":"/tmp/remem summarize --host claude-code","timeout":120}]}]}}"#,
        )?;
        let previous_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &path);
        Ok(Self {
            previous_home,
            path,
        })
    }
}

impl Drop for ScopedClaudeHome {
    fn drop(&mut self) {
        if let Some(previous) = self.previous_home.as_ref() {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

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

fn claude_startup_invocation(project: &str, session_id: &str) -> ContextInvocation {
    ContextInvocation {
        cwd: project.to_string(),
        project: project.to_string(),
        session_id: Some(session_id.to_string()),
        transcript_path: None,
        source: Some("startup".to_string()),
        host: HostKind::ClaudeCode,
        use_colors: false,
        debug: false,
        force: false,
        gate_mode: Some("auto".to_string()),
    }
}

fn assert_claude_hook_warning(output: &str) {
    assert!(output.contains("## Hook Integrity Warning"), "{output}");
    assert!(output.contains("3/6 registered"), "{output}");
    assert!(
        output.contains("remem install --target claude --repair"),
        "{output}"
    );
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
fn claude_hook_warning_survives_db_open_failure() -> anyhow::Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("context-hook-db-open");
    data_dir.remove_db_files();
    let _home = ScopedClaudeHome::with_incomplete_hooks("db-open")?;
    let project = data_dir.path.to_string_lossy().to_string();

    let output = generate_context_output_for_test(
        claude_startup_invocation(&project, "sess-hook-db-open"),
        true,
    )?;

    assert!(output.contains("failed to open remem database"), "{output}");
    assert_claude_hook_warning(&output);
    Ok(())
}

#[test]
fn claude_hook_warning_survives_context_gate_suppression() -> anyhow::Result<()> {
    let data_dir = crate::db::test_support::ScopedTestDataDir::new("context-hook-gate-suppress");
    let setup = crate::db::test_support::runtime_connection()?;
    drop(setup);
    let _home = ScopedClaudeHome::with_incomplete_hooks("gate-suppress")?;
    let project = data_dir.path.to_string_lossy().to_string();
    let invocation = claude_startup_invocation(&project, "sess-hook-gate-suppress");

    let first = generate_context_output_for_test(invocation.clone(), true)?;
    let second = generate_context_output_for_test(invocation, true)?;

    assert_claude_hook_warning(&first);
    assert_claude_hook_warning(&second);
    assert!(
        second.trim_start().starts_with("## Hook Integrity Warning"),
        "{second}"
    );
    let conn = Connection::open(data_dir.db_path())?;
    let (suppress_count, _) = gate_row(&conn, "sess-hook-gate-suppress")?;
    assert_eq!(suppress_count, 1);
    Ok(())
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

    let conn = Connection::open(data_dir.db_path())?;
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

    let conn = Connection::open(data_dir.db_path())?;
    let (first_suppress_count, first_data_version) =
        gate_row(&conn, "sess-gate-existing-load-error")?;
    assert_eq!(first_suppress_count, 0);
    assert!(first_data_version
        .as_deref()
        .is_some_and(|value| value.starts_with("v3:")));

    conn.execute("DROP TABLE memory_lessons", [])?;
    drop(conn);

    generate_context_for_test(invocation, true)?;

    let conn = Connection::open(data_dir.db_path())?;
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

    let conn = Connection::open(data_dir.db_path())?;
    let (first_suppress_count, first_data_version) = gate_row(&conn, "sess-gate-fts-load-error")?;
    assert_eq!(first_suppress_count, 0);
    assert!(first_data_version
        .as_deref()
        .is_some_and(|value| value.starts_with("v3:")));

    conn.execute("DROP TABLE memories_fts", [])?;
    drop(conn);

    generate_context_for_test(invocation, true)?;

    let conn = Connection::open(data_dir.db_path())?;
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
