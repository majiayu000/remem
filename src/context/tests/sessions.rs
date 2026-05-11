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
