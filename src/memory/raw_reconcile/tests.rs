use std::path::PathBuf;

use rusqlite::Connection;

use super::*;
use crate::ingest::sessions::{run_ingest_sessions, IngestOptions};

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "remem-reconcile-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&path).expect("create reconcile root");
        Self { path }
    }

    fn scan_root(&self) -> ScanRoot {
        ScanRoot {
            label: "fixture".to_string(),
            path: self.path.clone(),
            required: true,
        }
    }

    fn write(&self, name: &str, lines: &[&str]) -> PathBuf {
        let path = self.path.join(name);
        let mut content = lines.join("\n");
        content.push('\n');
        std::fs::write(&path, content).expect("write reconcile transcript");
        path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.path) {
            eprintln!("remove reconcile fixture failed: {error}");
        }
    }
}

fn setup() -> Connection {
    let conn = Connection::open_in_memory().expect("open reconcile database");
    crate::migrate::run_migrations(&conn).expect("migrate reconcile database");
    conn
}

fn ingest(conn: &Connection, root: &TempRoot) {
    let summary = run_ingest_sessions(conn, &[root.scan_root()], &IngestOptions::default())
        .expect("ingest reconcile fixture");
    assert_eq!(summary.failed_files, 0);
}

fn line(session: &str, role: &str, epoch: i64, text: &str) -> String {
    serde_json::json!({
        "type": role,
        "sessionId": session,
        "cwd": "/tmp/reconcile-project",
        "timestamp": epoch,
        "message": {"content": text}
    })
    .to_string()
}

#[test]
fn exact_reconcile_preserves_repeated_turns_and_counts_meta_xml() {
    let conn = setup();
    let root = TempRoot::new("exact");
    let user_one = line("stable-session", "user", 100, "repeat");
    let assistant = line("stable-session", "assistant", 101, "answer");
    let user_two = line("stable-session", "user", 102, "repeat");
    let meta = serde_json::json!({
        "type": "user",
        "sessionId": "stable-session",
        "cwd": "/tmp/reconcile-project",
        "timestamp": 103,
        "isMeta": true,
        "message": {"content": "metadata control"}
    })
    .to_string();
    let xml = line("stable-session", "user", 104, "<system>control</system>");
    let outside = line("stable-session", "assistant", 99, "outside");
    root.write(
        "stable-session.jsonl",
        &[&outside, &user_one, &assistant, &user_two, &meta, &xml],
    );
    ingest(&conn, &root);

    let report =
        reconcile_raw_archive(&conn, &[root.scan_root()], 100, 104).expect("reconcile exact");

    assert!(report.parity);
    assert_eq!(report.transcript.messages, 5);
    assert_eq!(report.transcript.user_messages, 4);
    assert_eq!(report.archive, report.transcript);
    assert_eq!(report.comparison.exact_sessions, 1);
    assert_eq!(report.intentional_exclusions.meta_user, 1);
    assert_eq!(report.intentional_exclusions.xml_control_user, 1);
    let serialized = serde_json::to_string(&report).expect("serialize aggregate report");
    assert!(!serialized.contains("reconcile-project"));
    assert!(!serialized.contains("stable-session"));
    assert!(!serialized.contains("repeat"));
    let human = render_reconcile_human(&report);
    for counter in [
        "exact_sessions=",
        "mismatch_sessions=",
        "transcript_only_messages=",
        "archive_only_messages=",
        "transcript_user=",
        "archive_assistant=",
        "conflicts=",
        "unsupported=",
        "fallback_time=",
        "unknown_time=",
        "malformed=",
    ] {
        assert!(human.contains(counter), "human output omitted {counter}");
    }
}

#[test]
fn equal_count_substitution_is_a_message_mismatch() {
    let conn = setup();
    let root = TempRoot::new("substitution");
    let user = line("substitution", "user", 100, "one");
    let assistant = line("substitution", "assistant", 101, "two");
    root.write("substitution.jsonl", &[&user, &assistant]);
    ingest(&conn, &root);
    conn.execute(
        "UPDATE raw_messages
         SET content_hash = ?1
         WHERE transcript_record_ordinal = 1",
        [crate::db::content_identity_hash(b"unrelated")],
    )
    .expect("substitute archive identity");

    let report = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 101)
        .expect("reconcile substitution");

    assert!(!report.parity);
    assert_eq!(report.transcript.messages, report.archive.messages);
    assert_eq!(report.comparison.message_mismatch_sessions, 1);
    assert_eq!(report.comparison.transcript_excess_messages, 1);
    assert_eq!(report.comparison.archive_excess_messages, 1);
}

#[test]
fn transcript_only_message_is_reported() {
    let conn = setup();
    let root = TempRoot::new("transcript-only");
    let user = line("transcript-only", "user", 100, "missing from archive");
    root.write("transcript-only.jsonl", &[&user]);
    ingest(&conn, &root);
    conn.execute("DELETE FROM raw_messages", [])
        .expect("remove archived occurrence");

    let report = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100)
        .expect("reconcile transcript-only fixture");

    assert_eq!(report.comparison.transcript_only_sessions, 1);
    assert_eq!(report.comparison.transcript_only_messages, 1);
    assert_eq!(report.transcript.messages, 1);
    assert_eq!(report.archive.messages, 0);
    assert!(!report.parity);
}

#[test]
fn malformed_record_is_counted_from_a_current_snapshot() {
    let conn = setup();
    let root = TempRoot::new("malformed");
    let path = root.write("malformed.jsonl", &["{not-json}"]);
    let plan = crate::ingest::session_identity::probe("fixture", &root.path, &path, None)
        .expect("probe malformed fixture path");
    let identity_id = crate::ingest::session_identity::upsert_claim(&conn, &plan, 1)
        .expect("persist malformed fixture ledger");
    crate::ingest::session_identity::mark_complete(
        &conn,
        identity_id,
        crate::ingest::session_identity::EventIndex {
            first_event_epoch: None,
            last_event_epoch: None,
            missing_event_time_count: 1,
        },
        1,
    )
    .expect("mark fixture ledger current");
    conn.execute(
        "INSERT INTO ingest_cursors (
            file_path, mtime_epoch, size_bytes, last_ingested_at
         ) VALUES (?1, ?2, ?3, 1)",
        rusqlite::params![
            format!("fixture\0{}", path.to_string_lossy()),
            plan.observed_mtime_ns / 1_000_000_000,
            plan.observed_size_bytes
        ],
    )
    .expect("persist fixture cursor");

    let report = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100)
        .expect("reconcile malformed fixture");

    assert_eq!(report.intentional_exclusions.malformed_record, 1);
    assert!(!report.parity);
}

#[test]
fn missing_event_time_is_selected_and_explained() {
    let conn = setup();
    let root = TempRoot::new("missing-time");
    let missing = serde_json::json!({
        "type": "user",
        "sessionId": "missing-time",
        "cwd": "/tmp/reconcile-project",
        "message": {"content": "no clock"}
    })
    .to_string();
    root.write("missing-time.jsonl", &[&missing]);
    ingest(&conn, &root);

    let report = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 200)
        .expect("reconcile missing event time");

    assert!(!report.parity);
    assert_eq!(report.intentional_exclusions.missing_event_time, 1);
    assert_eq!(
        report
            .intentional_exclusions
            .archive_ingest_fallback_event_time,
        1
    );
}

#[test]
fn stale_pre_capture_tuple_fails_before_reconciliation() {
    let conn = setup();
    let root = TempRoot::new("stale");
    let user = line("stale", "user", 100, "before");
    let path = root.write("stale.jsonl", &[&user]);
    ingest(&conn, &root);
    let mut content = std::fs::read_to_string(&path).expect("read stale fixture");
    content.push_str(&line("stale", "assistant", 101, "after"));
    content.push('\n');
    std::fs::write(path, content).expect("append stale fixture");

    let error = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 200)
        .expect_err("stale tuple must fail");

    assert!(error.to_string().contains("run `remem ingest-sessions`"));
}

#[test]
fn stale_out_of_window_file_blocks_bounded_reconciliation() {
    let conn = setup();
    let root = TempRoot::new("stale-outside");
    let outside = line("stale-outside", "user", 500, "outside");
    let path = root.write("stale-outside.jsonl", &[&outside]);
    ingest(&conn, &root);
    let mut content = std::fs::read_to_string(&path).expect("read outside fixture");
    content.push_str(&line("stale-outside", "assistant", 501, "still outside"));
    content.push('\n');
    std::fs::write(path, content).expect("append outside fixture");

    let error = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 200)
        .expect_err("every discovered transcript must have a fresh snapshot tuple");

    assert!(error.to_string().contains("run `remem ingest-sessions`"));
}

#[test]
fn deleted_file_from_optional_root_blocks_reconciliation() {
    let conn = setup();
    let root = TempRoot::new("deleted-optional");
    let user = line("deleted-optional", "user", 100, "must stay accounted");
    let path = root.write("deleted-optional.jsonl", &[&user]);
    ingest(&conn, &root);
    std::fs::remove_file(path).expect("delete indexed transcript");
    let mut optional = root.scan_root();
    optional.required = false;

    let error = reconcile_raw_archive(&conn, &[optional], 100, 100)
        .expect_err("optional roots with ledger history cannot hide deleted files");

    assert!(error.to_string().contains("run `remem ingest-sessions`"));
}

#[test]
fn missing_required_root_fails_loudly() {
    let conn = setup();
    let root = ScanRoot {
        label: "required".to_string(),
        path: std::env::temp_dir().join(format!(
            "remem-missing-required-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )),
        required: true,
    };

    let error = reconcile_raw_archive(&conn, &[root], 100, 100)
        .expect_err("missing required root must fail");

    assert!(error.to_string().contains("transcript discovery failed"));
}

#[test]
fn missing_cursor_blocks_reconciliation() {
    let conn = setup();
    let root = TempRoot::new("missing-cursor");
    let user = line("missing-cursor", "user", 100, "cursor required");
    root.write("missing-cursor.jsonl", &[&user]);
    ingest(&conn, &root);
    conn.execute("DELETE FROM ingest_cursors", [])
        .expect("remove cursor");

    let error = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100)
        .expect_err("contract v1 without a matching cursor is stale");

    assert!(error.to_string().contains("run `remem ingest-sessions`"));
}

#[test]
fn timestamped_unsupported_records_participate_in_candidate_bounds() {
    let conn = setup();
    let root = TempRoot::new("unsupported-bounds");
    let unsupported = serde_json::json!({
        "type": "progress",
        "sessionId": "unsupported-bounds",
        "timestamp": 150,
        "payload": {"status": "working"}
    })
    .to_string();
    root.write("unsupported-bounds.jsonl", &[&unsupported]);
    ingest(&conn, &root);

    let report = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 200)
        .expect("timestamped unsupported record should select the transcript");

    assert!(report.parity);
    assert_eq!(report.intentional_exclusions.unsupported_record, 1);
    assert_eq!(report.transcript.messages, 0);
    assert_eq!(report.archive.messages, 0);
}

#[test]
fn conflicts_are_scoped_to_selected_window_identities() {
    let conn = setup();
    let root = TempRoot::new("conflict-window");
    let inside = line("inside", "user", 100, "inside");
    let outside = line("outside", "user", 500, "outside");
    let inside_path = root.write("inside.jsonl", &[&inside]);
    let outside_path = root.write("outside.jsonl", &[&outside]);
    ingest(&conn, &root);
    conn.execute(
        "UPDATE raw_session_identities SET status = 'conflict',
             conflict_reason = 'test'
         WHERE transcript_path = ?1",
        [outside_path.to_string_lossy().as_ref()],
    )
    .expect("mark outside conflict");

    let first =
        reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100).expect("window reconcile");
    assert_eq!(first.comparison.identity_conflicts, 0);

    conn.execute(
        "UPDATE raw_session_identities SET status = 'conflict',
             conflict_reason = 'test'
         WHERE transcript_path = ?1",
        [inside_path.to_string_lossy().as_ref()],
    )
    .expect("mark inside conflict");
    let second =
        reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100).expect("conflict reconcile");
    assert_eq!(second.comparison.identity_conflicts, 1);
    assert!(!second.parity);
}

#[test]
fn production_contract_zero_conflict_is_counted_instead_of_rejected_as_stale() {
    let conn = setup();
    let root = TempRoot::new("real-conflict");
    std::fs::create_dir_all(root.path.join("first")).expect("create first alias directory");
    std::fs::create_dir_all(root.path.join("second")).expect("create second alias directory");
    let first = line("canonical-one", "user", 100, "first claim");
    let second = line("canonical-two", "user", 100, "second claim");
    root.write("first/shared.jsonl", &[&first]);
    root.write("second/shared.jsonl", &[&second]);

    let summary = run_ingest_sessions(&conn, &[root.scan_root()], &IngestOptions::default())
        .expect("ingest conflicting fallback group");
    assert_eq!(summary.failed_files, 2);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM raw_session_identities
             WHERE status = 'conflict' AND contract_version = 0",
            [],
            |row| row.get::<_, i64>(0)
        )
        .expect("count production conflict rows"),
        2
    );

    let report = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100)
        .expect("current conflict snapshot should produce an aggregate report");

    assert_eq!(report.comparison.identity_conflicts, 1);
    assert!(!report.parity);
}

#[test]
fn raw_rows_without_discoverable_transcripts_remain_archive_only_or_explained() {
    let conn = setup();
    crate::memory::raw_archive::insert_raw_message_from_root_at(
        &conn,
        "historical",
        "project",
        "user",
        "windowed historical row",
        crate::memory::raw_archive::SOURCE_TRANSCRIPT,
        None,
        None,
        "local",
        Some(100),
    )
    .expect("insert windowed historical row");
    crate::memory::raw_archive::insert_raw_message_from_root_at(
        &conn,
        "fallback",
        "project",
        "assistant",
        "timestamp fallback row",
        crate::memory::raw_archive::SOURCE_TRANSCRIPT,
        None,
        None,
        "local",
        None,
    )
    .expect("insert fallback-time row");
    crate::memory::raw_archive::insert_raw_message_from_root_at(
        &conn,
        "manual",
        "project",
        "user",
        "unknown-time row",
        crate::memory::raw_archive::SOURCE_MANUAL,
        None,
        None,
        "local",
        Some(100),
    )
    .expect("insert unknown-time row");

    let report =
        reconcile_raw_archive(&conn, &[], 100, 100).expect("reconcile archive without transcripts");

    assert_eq!(report.archive.sessions, 1);
    assert_eq!(report.archive.messages, 1);
    assert_eq!(report.comparison.archive_only_sessions, 1);
    assert_eq!(report.comparison.archive_only_messages, 1);
    assert_eq!(
        report
            .intentional_exclusions
            .archive_ingest_fallback_event_time,
        1
    );
    assert_eq!(report.intentional_exclusions.archive_unknown_event_time, 1);
    assert!(!report.parity);
}

#[test]
fn unchanged_inputs_produce_byte_stable_reports() {
    let conn = setup();
    let root = TempRoot::new("deterministic");
    let user = line("deterministic", "user", 100, "stable");
    root.write("deterministic.jsonl", &[&user]);
    ingest(&conn, &root);

    let first = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100)
        .expect("first deterministic report");
    let second = reconcile_raw_archive(&conn, &[root.scan_root()], 100, 100)
        .expect("second deterministic report");

    assert_eq!(
        serde_json::to_vec(&first).expect("serialize first report"),
        serde_json::to_vec(&second).expect("serialize second report")
    );
}

#[test]
fn post_capture_append_is_outside_reconciliation_boundary() {
    let conn = setup();
    let root = TempRoot::new("bounded");
    let before = line("bounded", "user", 100, "before capture");
    let path = root.write("bounded.jsonl", &[&before]);
    ingest(&conn, &root);
    let captured =
        capture_candidates(&conn, &[root.scan_root()], 100, 101).expect("capture candidates");
    let mut content = std::fs::read_to_string(&path).expect("read bounded fixture");
    content.push_str(&line(
        "bounded",
        "assistant",
        101,
        "private post-capture sentinel",
    ));
    content.push('\n');
    std::fs::write(&path, content).expect("append after capture");

    let report =
        reconcile_captured(&conn, captured, 100, 101).expect("reconcile captured boundary");

    assert!(report.parity);
    assert_eq!(report.transcript.messages, 1);
    let json = serde_json::to_string(&report).expect("serialize bounded report");
    assert!(!json.contains("private post-capture sentinel"));
}

#[test]
fn inverted_window_is_rejected() {
    let conn = setup();
    let error = reconcile_raw_archive(&conn, &[], 200, 100).expect_err("inverted window must fail");
    assert!(error.to_string().contains("--since must be <= --until"));
}
