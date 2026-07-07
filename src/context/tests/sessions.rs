use rusqlite::Connection;

use super::super::query::query_recent_summaries;
use super::{create_session_summary_schema, insert_session_summary};

#[test]
fn query_recent_summaries_filters_self_diagnostics_and_backfills() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";

    insert_session_summary(
        &conn,
        project,
        "Debug remem context memory injection",
        Some("SessionStart memories loaded investigation"),
        300,
    );
    insert_session_summary(
        &conn,
        project,
        "Fix runtime hook",
        Some("Validated hook behavior"),
        299,
    );
    insert_session_summary(
        &conn,
        project,
        "Analyze SessionStart memories loaded",
        None,
        298,
    );
    insert_session_summary(
        &conn,
        project,
        "Review PR install paths",
        Some("Checked install scripts"),
        297,
    );
    insert_session_summary(
        &conn,
        project,
        "Memory injection follow-up",
        Some("remem context smoke test"),
        296,
    );
    insert_session_summary(
        &conn,
        project,
        "Repair guard source path",
        Some("Added source path evidence"),
        295,
    );

    let summaries = query_recent_summaries(&conn, project, 3).unwrap();

    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].request, "Fix runtime hook");
    assert_eq!(summaries[1].request, "Review PR install paths");
    assert_eq!(summaries[2].request, "Repair guard source path");
}

#[test]
fn query_recent_summaries_scans_past_self_diagnostic_burst() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";

    for idx in 0..30 {
        insert_session_summary(
            &conn,
            project,
            &format!("Debug remem context memory injection {idx}"),
            Some("SessionStart memories loaded investigation"),
            1_000 - idx,
        );
    }
    insert_session_summary(
        &conn,
        project,
        "Fix runtime hook",
        Some("Validated hook behavior"),
        100,
    );
    insert_session_summary(
        &conn,
        project,
        "Repair guard source path",
        Some("Added source path evidence"),
        99,
    );

    let summaries = query_recent_summaries(&conn, project, 2).unwrap();

    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].request, "Fix runtime hook");
    assert_eq!(summaries[1].request, "Repair guard source path");
}

#[test]
fn query_recent_summaries_suppresses_stale_design_prototype_noise() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_session_summary(
        &conn,
        project,
        "Build landing page and wireframe variants",
        Some("Starfield prototype shipped"),
        now - 8 * 86400,
    );
    insert_session_summary(
        &conn,
        project,
        "Generate VibeGuard wireframe prototype",
        Some("Landing page assets updated"),
        now - 9 * 86400,
    );
    insert_session_summary(
        &conn,
        project,
        "Fix runtime hook",
        Some("Validated hook behavior"),
        now - 10 * 86400,
    );

    let summaries = query_recent_summaries(&conn, project, 5).unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].request, "Fix runtime hook");
}

#[test]
fn query_recent_summaries_keeps_stale_design_summary_as_last_resort() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_session_summary(
        &conn,
        project,
        "Build landing page and wireframe variants",
        Some("Starfield prototype shipped"),
        now - 8 * 86400,
    );

    let summaries = query_recent_summaries(&conn, project, 5).unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].request,
        "Build landing page and wireframe variants"
    );
}

#[test]
fn query_recent_summaries_allows_normal_summary_after_low_signal_same_cluster() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/vibeguard";
    let now = chrono::Utc::now().timestamp();

    insert_session_summary(
        &conn,
        project,
        "Review release work",
        Some("Starfield prototype shipped"),
        now - 8 * 86400,
    );
    insert_session_summary(
        &conn,
        project,
        "Review release work",
        Some("Validated current runtime hook behavior"),
        now - 9 * 86400,
    );

    let summaries = query_recent_summaries(&conn, project, 5).unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(
        summaries[0].completed.as_deref(),
        Some("Validated current runtime hook behavior")
    );
}

#[test]
fn query_recent_summaries_excludes_tool_and_domain_owned_rows() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/stash";

    insert_session_summary(
        &conn,
        project,
        "Legacy Stash session",
        Some("Legacy project rows remain compatible"),
        300,
    );
    insert_owned_summary(
        &conn,
        project,
        "Repo Stash session",
        "repo",
        project,
        Some(project),
        301,
    );
    insert_owned_summary(
        &conn,
        project,
        "Codex sandbox session",
        "tool",
        "codex-cli",
        None,
        302,
    );
    insert_owned_summary(
        &conn,
        project,
        "Grok API session",
        "domain",
        "grok-api",
        None,
        303,
    );

    let summaries = query_recent_summaries(&conn, project, 10).unwrap();
    let requests = summaries
        .iter()
        .map(|summary| summary.request.as_str())
        .collect::<Vec<_>>();

    assert_eq!(requests, vec!["Repo Stash session", "Legacy Stash session"]);
}

#[test]
fn query_recent_summaries_uses_semantic_rollup_rows_without_synthetic_noise() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/remem";

    insert_session_summary(
        &conn,
        project,
        "Legacy user-facing summary",
        Some("Legacy summary remains context visible"),
        300,
    );
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch, session_row_id,
          covered_from_event_id, covered_to_event_id)
         VALUES ('capture-rollup-10', ?1, 'Captured event range 1..3',
                 'Capture rollup text', 301, 10, 1, 3)",
        rusqlite::params![project],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch, session_row_id,
          covered_from_event_id, covered_to_event_id)
         VALUES ('capture-rollup-11', ?1, 'Retire legacy Summary writer',
                 'Rollup now carries semantic fields', 302, 11, 4, 6)",
        rusqlite::params![project],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, decisions, created_at_epoch,
          session_row_id, covered_from_event_id, covered_to_event_id)
         VALUES ('capture-rollup-12', ?1, 'Captured event range 7..9',
                 'Rollup summary without request', 'Rollup structured decision',
                 303, 12, 7, 9)",
        rusqlite::params![project],
    )
    .unwrap();

    let summaries = query_recent_summaries(&conn, project, 10).unwrap();
    let requests = summaries
        .iter()
        .map(|summary| summary.request.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        requests,
        vec![
            "Rollup structured decision",
            "Retire legacy Summary writer",
            "Legacy user-facing summary"
        ]
    );
    assert!(!requests.contains(&"Captured event range 1..3"));
    assert!(!requests.contains(&"Captured event range 7..9"));
}

#[test]
fn query_recent_summaries_dedupes_session_identity_before_limit() {
    let conn = Connection::open_in_memory().unwrap();
    create_session_summary_schema(&conn);
    let project = "/tmp/remem";

    conn.execute(
        "INSERT INTO sessions (id, session_id) VALUES (20, 'abcdefgh-captured')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_summaries
         (memory_session_id, project, request, completed, created_at_epoch, session_row_id,
          covered_from_event_id, covered_to_event_id)
         VALUES
         ('capture-rollup-20', ?1, 'Newest rollup range',
          'First visible rollup', 303, 20, 4, 6),
         ('capture-rollup-20', ?1, 'Older rollup range',
          'Same captured session', 302, 20, 1, 3),
         ('mem-abcdefgh', ?1, 'Legacy Summary phrasing',
          'Same dual-written session', 301, NULL, NULL, NULL),
         ('session-other', ?1, 'Separate session',
          'Must survive the limit', 300, NULL, NULL, NULL)",
        rusqlite::params![project],
    )
    .unwrap();

    let summaries = query_recent_summaries(&conn, project, 2).unwrap();
    let requests = summaries
        .iter()
        .map(|summary| summary.request.as_str())
        .collect::<Vec<_>>();

    assert_eq!(requests, vec!["Newest rollup range", "Separate session"]);
}

fn insert_owned_summary(
    conn: &Connection,
    project: &str,
    request: &str,
    owner_scope: &str,
    owner_key: &str,
    target_project: Option<&str>,
    created_at_epoch: i64,
) {
    conn.execute(
        "INSERT INTO session_summaries
         (project, request, completed, created_at_epoch, source_project,
          target_project, owner_scope, owner_key)
         VALUES (?1, ?2, 'done', ?3, ?1, ?4, ?5, ?6)",
        rusqlite::params![
            project,
            request,
            created_at_epoch,
            target_project,
            owner_scope,
            owner_key
        ],
    )
    .unwrap();
}
