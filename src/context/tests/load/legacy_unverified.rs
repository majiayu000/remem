use rusqlite::Connection;

use crate::memory::lesson::{save_lesson, SaveLessonRequest};

use super::super::super::query::load_context_data;
use super::super::{insert_memory, setup_context_schema};

#[test]
fn legacy_unverified_ordinary_and_lesson_rows_are_audited_before_sessionstart() {
    let conn = Connection::open_in_memory().unwrap();
    setup_context_schema(&conn);
    let project = "/tmp/remem-g2";
    let now = chrono::Utc::now().timestamp();
    insert_memory(
        &conn,
        901,
        project,
        None,
        "bugfix",
        "legacy ordinary",
        "legacy ordinary payload",
        now,
    );
    conn.execute(
        "UPDATE memories
         SET source_trust_class = 'local_tool_output', source_candidate_id = NULL,
             evidence_event_ids = NULL, confidence = NULL, valid_from_epoch = NULL,
             state_key_id = NULL
         WHERE id = 901",
        [],
    )
    .unwrap();
    let lesson_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("g2"),
            project,
            topic_key: Some("legacy-lesson"),
            title: "legacy lesson",
            content: "Lesson: legacy lesson payload",
            confidence: 0.9,
            source_evidence: Some("evidence removed below"),
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: Some(now),
            stale_after_epoch: None,
        },
    )
    .unwrap();
    conn.execute(
        "UPDATE memory_lessons SET source_evidence = '' WHERE memory_id = ?1",
        [lesson_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE memories SET source_trust_class = 'local_tool_output' WHERE id = ?1",
        [lesson_id],
    )
    .unwrap();

    let loaded = load_context_data(&conn, project, None);
    assert!(loaded.memories.iter().all(|memory| memory.id != 901));
    assert!(loaded
        .lessons
        .iter()
        .all(|lesson| lesson.memory.id != lesson_id));
    let reasons = loaded
        .preselection_drops
        .iter()
        .map(|drop| {
            let id = match &drop.item {
                crate::context::types::ContextPreselectionItem::Memory(memory) => memory.id,
                _ => -1,
            };
            (id, drop.reason)
        })
        .collect::<Vec<_>>();
    assert!(reasons.contains(&(901, "legacy_unverified_provenance_missing")));
    assert!(reasons.contains(&(lesson_id, "legacy_unverified_provenance_missing")));
}
