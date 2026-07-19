use crate::memory::lesson::{save_lesson, LessonMemory, LessonMetadata, SaveLessonRequest};
use crate::memory::memory_staleness_label_for_anchor;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

use super::super::audit::build_context_audit_items;
use super::super::query::load_context_data;
use super::super::relevance::SessionStartRelevancePlan;
use super::super::sections::render_lessons_with_limit_and_staleness;
use super::super::types::ContextDiagnostics;
use super::{insert_memory, setup_context_schema};

#[test]
fn load_context_data_marks_source_anchor_failures_as_errors() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    setup_context_git_trace_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        101,
        project,
        Some("legacy-bad-files"),
        "decision",
        "Legacy bad files",
        "Malformed legacy files should only affect this memory.",
        now,
    );
    conn.execute(
        "UPDATE memories
         SET session_id = 'bad-session', files = '[not-json', branch = 'main'
         WHERE id = 101",
        [],
    )
    .unwrap();
    insert_memory(
        &conn,
        102,
        project,
        Some("stale-source-anchor"),
        "decision",
        "Stale source anchor",
        "This memory should keep its verify-before-trust warning.",
        now - 1,
    );
    conn.execute(
        "UPDATE memories
         SET session_id = 'stale-session', files = ?1, branch = 'main'
         WHERE id = 102",
        [r#"["src/stale.rs"]"#],
    )
    .unwrap();
    link_context_commit(
        &conn,
        1,
        project,
        "source-stale",
        100,
        &["src/stale.rs"],
        "stale-session",
    );
    insert_context_commit(
        &conn,
        2,
        project,
        "later-stale",
        200,
        &["src/stale.rs"],
        Some("main"),
    );

    let loaded = load_context_data(&conn, project, Some("main"));

    assert_eq!(
        loaded
            .staleness_labels
            .get(&101)
            .map(|label| label.source_anchor.as_str()),
        Some("error")
    );
    assert_eq!(
        loaded
            .staleness_labels
            .get(&102)
            .map(|label| label.source_anchor.as_str()),
        Some("verify-before-trust")
    );
    assert!(loaded.errors.iter().any(|error| {
        error.section == "staleness" && error.message.contains("source-anchor staleness")
    }));
}

#[test]
fn load_context_data_includes_lesson_memories_in_staleness_labels() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    setup_context_git_trace_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    let lesson_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("lesson-session"),
            project,
            topic_key: Some("lesson-source-anchor"),
            title: "Check lesson anchors",
            content: "Lesson: lesson memories should render source-anchor warnings too.",
            confidence: 0.9,
            source_evidence: Some("lesson evidence"),
            files: Some(r#"["src/lesson.rs"]"#),
            branch: Some("main"),
            scope: "project",
            created_at_epoch: Some(now),
            stale_after_epoch: None,
        },
    )
    .unwrap();
    link_context_commit(
        &conn,
        1,
        project,
        "source-lesson",
        100,
        &["src/lesson.rs"],
        "lesson-session",
    );
    insert_context_commit(
        &conn,
        2,
        project,
        "later-lesson",
        200,
        &["src/lesson.rs"],
        Some("main"),
    );

    let loaded = load_context_data(&conn, project, Some("main"));

    assert_eq!(loaded.lessons.len(), 1);
    assert_eq!(loaded.lessons[0].memory.id, lesson_id);
    assert_eq!(
        loaded
            .staleness_labels
            .get(&lesson_id)
            .map(|label| label.source_anchor.as_str()),
        Some("verify-before-trust")
    );
}

#[test]
fn render_lessons_includes_source_anchor_staleness_labels() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let lessons = vec![sample_lesson(1, "Stale lesson", 0.9, 2)];
    let mut labels = HashMap::new();
    labels.insert(
        1,
        memory_staleness_label_for_anchor(&lessons[0].memory, now, "verify-before-trust"),
    );

    let rendered =
        render_lessons_with_limit_and_staleness(&mut output, &lessons, 1, 240, now, &labels);

    assert_eq!(rendered, 1);
    assert!(output.contains("Stale lesson"));
    assert!(output.contains("source_anchor=verify-before-trust"));
}

#[test]
fn context_audit_uses_rendered_source_anchor_labels() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    setup_context_git_trace_schema(&conn);
    let project = "/tmp/remem";
    let now = chrono::Utc::now().timestamp();

    insert_memory(
        &conn,
        201,
        project,
        Some("audit-source-anchor"),
        "decision",
        "Audit source anchor",
        "Audit items should store the rendered source-anchor label.",
        now,
    );
    conn.execute(
        "UPDATE memories
         SET session_id = 'audit-session', files = ?1, branch = 'main'
         WHERE id = 201",
        [r#"["src/audit.rs"]"#],
    )
    .unwrap();
    link_context_commit(
        &conn,
        1,
        project,
        "source-audit",
        100,
        &["src/audit.rs"],
        "audit-session",
    );
    insert_context_commit(
        &conn,
        2,
        project,
        "later-audit",
        200,
        &["src/audit.rs"],
        Some("main"),
    );

    let mut loaded = load_context_data(&conn, project, Some("main"));
    loaded.memories.retain(|memory| memory.id == 201);
    loaded.lessons.clear();
    loaded.workstreams.clear();
    loaded.summaries.clear();
    loaded.diagnostics = ContextDiagnostics::default();
    let relevance = SessionStartRelevancePlan::disabled(&[]);
    let audit_items = build_context_audit_items(
        &loaded,
        &[201],
        &[],
        &[],
        &[],
        &[],
        &relevance,
        &HashSet::new(),
    );

    assert_eq!(audit_items.len(), 2);
    let core = audit_items
        .iter()
        .find(|item| item.channel == "core")
        .expect("core audit item");
    assert_eq!(core.status, "injected");
    assert!(core.staleness.contains("source_anchor=verify-before-trust"));
}

fn sample_lesson(id: i64, title: &str, confidence: f64, reinforcement_count: i64) -> LessonMemory {
    let memory = super::sample_memory(id, "lesson", title);
    LessonMemory {
        memory,
        metadata: LessonMetadata {
            memory_id: id,
            confidence,
            reinforcement_count,
            source_evidence: None,
            last_reinforced_at_epoch: 1_710_000_000,
            stale_after_epoch: None,
            outcome_kind: "unknown".to_string(),
            success_count: 0,
            failure_count: 0,
            recovery_count: 0,
            correction_count: 0,
            revert_count: 0,
        },
    }
}

fn setup_context_git_trace_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE git_commits (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            repo_path TEXT NOT NULL,
            sha TEXT NOT NULL,
            short_sha TEXT NOT NULL,
            branch TEXT,
            message TEXT,
            authored_at_epoch INTEGER,
            changed_files TEXT NOT NULL DEFAULT '[]',
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
        );
        CREATE TABLE git_commit_sessions (
            commit_id INTEGER NOT NULL,
            session_id TEXT NOT NULL,
            memory_session_id TEXT,
            source TEXT NOT NULL,
            linked_at_epoch INTEGER NOT NULL,
            PRIMARY KEY(commit_id, session_id)
        );",
    )
    .unwrap();
}

fn link_context_commit(
    conn: &Connection,
    id: i64,
    project: &str,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    memory_session_id: &str,
) {
    insert_context_commit(conn, id, project, sha, epoch, changed_files, Some("main"));
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, ?2, ?3, 'test', ?4)",
        params![id, format!("content-{id}"), memory_session_id, epoch],
    )
    .unwrap();
}

fn insert_context_commit(
    conn: &Connection,
    id: i64,
    project: &str,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    branch: Option<&str>,
) {
    let changed_files = serde_json::to_string(changed_files).unwrap();
    conn.execute(
        "INSERT INTO git_commits
         (id, project, repo_path, sha, short_sha, branch, message,
          authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?2, ?3, ?3, ?4, NULL, ?5, ?6, ?5, ?5)",
        params![id, project, sha, branch, epoch, changed_files],
    )
    .unwrap();
}
