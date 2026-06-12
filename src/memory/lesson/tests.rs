use rusqlite::{params, Connection};

use super::*;
use crate::memory::types::tests_helper::setup_memory_schema;

#[test]
fn save_lesson_creates_metadata() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let id = save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project: "/repo",
            topic_key: Some("lesson-build-failure"),
            title: "Avoid build failure loops",
            content: "Lesson: after repeated build failures, stop and challenge the hypothesis.",
            confidence: 0.8,
            source_evidence: Some("cargo check failed twice"),
            files: None,
            branch: Some("main"),
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )
    .unwrap();

    let metadata = get_lesson_metadata(&conn, id)
        .unwrap()
        .expect("metadata should exist");
    assert_eq!(metadata.memory_id, id);
    assert_eq!(metadata.reinforcement_count, 1);
    assert_eq!(
        metadata.source_evidence.as_deref(),
        Some("cargo check failed twice")
    );
}

#[test]
fn save_lesson_reinforces_duplicate_topic() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let req = SaveLessonRequest {
        session_id: Some("s1"),
        project: "/repo",
        topic_key: Some("lesson-build-failure"),
        title: "Avoid build failure loops",
        content: "Lesson: after repeated build failures, stop and challenge the hypothesis.",
        confidence: 0.6,
        source_evidence: Some("first run"),
        files: None,
        branch: None,
        scope: "project",
        created_at_epoch: None,
        stale_after_epoch: None,
    };
    let first_id = save_lesson(&conn, &req).unwrap();

    let second_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            confidence: 0.9,
            source_evidence: Some("second run"),
            ..req
        },
    )
    .unwrap();

    assert_eq!(first_id, second_id);
    let metadata = get_lesson_metadata(&conn, first_id).unwrap().unwrap();
    assert_eq!(metadata.reinforcement_count, 2);
    assert_eq!(metadata.confidence, 0.9);
    assert_eq!(metadata.source_evidence.as_deref(), Some("second run"));
}

#[test]
fn save_lesson_reinforces_semantic_near_duplicate() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let first_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project: "/repo",
            topic_key: Some("lesson-build-loop-a"),
            title: "Avoid build failure loops",
            content: "Lesson: after repeated build failures, stop and challenge the hypothesis before editing again.",
            confidence: 0.6,
            source_evidence: Some("first run"),
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )
    ?;
    let second_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s2"),
            project: "/repo",
            topic_key: Some("lesson-build-loop-b"),
            title: "Break repeated build failures",
            content: "Lesson: when build failures repeat, pause and challenge the hypothesis before more edits.",
            confidence: 0.9,
            source_evidence: Some("second run"),
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )
    ?;

    assert_eq!(second_id, first_id);
    let metadata =
        get_lesson_metadata(&conn, first_id)?.ok_or_else(|| anyhow::anyhow!("metadata missing"))?;
    assert_eq!(metadata.reinforcement_count, 2);
    assert_eq!(metadata.confidence, 0.9);
    assert_eq!(metadata.source_evidence.as_deref(), Some("second run"));
    Ok(())
}

#[test]
fn semantic_lesson_dedup_keeps_branch_scope_isolated() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);

    let first_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project: "/repo",
            topic_key: Some("lesson-build-loop-main"),
            title: "Avoid build failure loops",
            content: "Lesson: after repeated build failures, stop and challenge the hypothesis before editing again.",
            confidence: 0.6,
            source_evidence: Some("main branch"),
            files: None,
            branch: Some("main"),
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )?;
    let second_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s2"),
            project: "/repo",
            topic_key: Some("lesson-build-loop-feature"),
            title: "Break repeated build failures",
            content: "Lesson: when build failures repeat, pause and challenge the hypothesis before more edits.",
            confidence: 0.9,
            source_evidence: Some("feature branch"),
            files: None,
            branch: Some("feature"),
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )?;

    assert_ne!(second_id, first_id);
    let active_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories
         WHERE project = '/repo'
           AND memory_type = 'lesson'
           AND status = 'active'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(active_count, 2);
    Ok(())
}

#[test]
fn save_lesson_reinforcement_clears_stale_deadline() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let now = chrono::Utc::now().timestamp();

    let req = SaveLessonRequest {
        session_id: Some("s1"),
        project: "/repo",
        topic_key: Some("lesson-stale-refresh"),
        title: "Refresh stale lesson",
        content:
            "Lesson: a reinforced lesson should become usable again when it is no longer stale.",
        confidence: 0.7,
        source_evidence: Some("first run"),
        files: None,
        branch: None,
        scope: "project",
        created_at_epoch: None,
        stale_after_epoch: Some(now - 1),
    };
    let id = save_lesson(&conn, &req).unwrap();
    assert!(list_lessons_for_context(&conn, "/repo", None, 10)
        .unwrap()
        .is_empty());

    let refreshed_id = save_lesson(
        &conn,
        &SaveLessonRequest {
            source_evidence: Some("second run"),
            stale_after_epoch: None,
            ..req
        },
    )
    .unwrap();

    assert_eq!(id, refreshed_id);
    let metadata = get_lesson_metadata(&conn, id).unwrap().unwrap();
    assert_eq!(metadata.stale_after_epoch, None);
    let lessons = list_lessons_for_context(&conn, "/repo", None, 10).unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].memory.title, "Refresh stale lesson");
}

#[test]
fn list_lessons_for_context_filters_low_confidence_and_stale() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    let now = chrono::Utc::now().timestamp();

    for (idx, confidence, stale_after_epoch, content) in [
        (
            1,
            0.9,
            None,
            "Lesson: active evidence-backed fixes need fresh verification output.",
        ),
        (
            2,
            0.2,
            None,
            "Lesson: low confidence notes should stay below the context threshold.",
        ),
        (
            3,
            0.8,
            Some(now - 1),
            "Lesson: stale remediation notes expire after their review window.",
        ),
    ] {
        save_lesson(
            &conn,
            &SaveLessonRequest {
                session_id: Some("s1"),
                project: "/repo",
                topic_key: Some(&format!("lesson-{idx}")),
                title: &format!("Lesson {idx}"),
                content,
                confidence,
                source_evidence: None,
                files: None,
                branch: None,
                scope: "project",
                created_at_epoch: None,
                stale_after_epoch,
            },
        )
        .unwrap();
    }

    let lessons = list_lessons_for_context(&conn, "/repo", None, 10).unwrap();
    assert_eq!(lessons.len(), 1);
    assert_eq!(lessons[0].memory.title, "Lesson 1");
}

#[test]
fn list_lessons_for_context_filters_other_branches() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    for (title, branch) in [
        ("Main branch lesson", Some("main")),
        ("Feature branch lesson", Some("feature/search")),
        ("Branchless lesson", None),
    ] {
        save_lesson(
            &conn,
            &SaveLessonRequest {
                session_id: Some("s1"),
                project: "/repo",
                topic_key: Some(&title.replace(' ', "-").to_lowercase()),
                title,
                content: "Lesson: branch-specific context should not leak into unrelated branches.",
                confidence: 0.8,
                source_evidence: None,
                files: None,
                branch,
                scope: "project",
                created_at_epoch: None,
                stale_after_epoch: None,
            },
        )
        .unwrap();
    }

    let mut titles: Vec<_> = list_lessons_for_context(&conn, "/repo", Some("main"), 10)
        .unwrap()
        .into_iter()
        .map(|lesson| lesson.memory.title)
        .collect();
    titles.sort();
    assert_eq!(titles, vec!["Branchless lesson", "Main branch lesson"]);

    let all_titles: Vec<_> = list_lessons_for_context(&conn, "/repo", None, 10)
        .unwrap()
        .into_iter()
        .map(|lesson| lesson.memory.title)
        .collect();
    assert!(all_titles.contains(&"Feature branch lesson".to_string()));
}

#[test]
fn list_lessons_for_context_keeps_project_before_global_scope() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project: "/global",
            topic_key: Some("lesson-global"),
            title: "Global lesson",
            content: "Lesson: use the verified workflow when every project hits this failure.",
            confidence: 0.99,
            source_evidence: None,
            files: None,
            branch: None,
            scope: "global",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )
    .unwrap();
    save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project: "/other",
            topic_key: Some("lesson-other-project"),
            title: "Other project lesson",
            content: "Lesson: this project-local item should not leak into unrelated projects.",
            confidence: 0.95,
            source_evidence: None,
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )
    .unwrap();
    save_lesson(
        &conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project: "/repo",
            topic_key: Some("lesson-project"),
            title: "Project lesson",
            content: "Lesson: prefer the project-specific workflow before global advice.",
            confidence: 0.6,
            source_evidence: None,
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )
    .unwrap();

    let lessons = list_lessons_for_context(&conn, "/repo", None, 10).unwrap();
    let titles: Vec<_> = lessons
        .iter()
        .map(|lesson| lesson.memory.title.as_str())
        .collect();

    assert_eq!(titles, vec!["Project lesson", "Global lesson"]);
}

#[test]
fn list_lessons_for_context_ranks_confidence_reinforcement_and_recency() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let high_confidence = save_ranked_lesson(&conn, "high-confidence", 0.9).unwrap();
    let older_reinforced = save_ranked_lesson(&conn, "older-reinforced", 0.7).unwrap();
    let newer_reinforced = save_ranked_lesson(&conn, "newer-reinforced", 0.7).unwrap();
    let lower_reinforced = save_ranked_lesson(&conn, "lower-reinforced", 0.7).unwrap();
    set_lesson_rank(&conn, high_confidence, 1, 100);
    set_lesson_rank(&conn, older_reinforced, 3, 100);
    set_lesson_rank(&conn, newer_reinforced, 3, 200);
    set_lesson_rank(&conn, lower_reinforced, 2, 300);

    let lessons = list_lessons_for_context(&conn, "/repo", None, 10).unwrap();
    let titles: Vec<_> = lessons
        .iter()
        .map(|lesson| lesson.memory.title.as_str())
        .collect();

    assert_eq!(
        titles,
        vec![
            "high-confidence",
            "newer-reinforced",
            "older-reinforced",
            "lower-reinforced"
        ]
    );
}

#[test]
fn is_lesson_candidate_requires_actionable_signal() {
    assert!(is_lesson_candidate(
        "Root cause: unchecked fallback hid the real error; avoid warning-only degradation."
    ));
    assert!(!is_lesson_candidate(
        "FTS5 trigram tokenizer handles CJK without word boundaries"
    ));
}

fn save_ranked_lesson(conn: &Connection, title: &str, confidence: f64) -> anyhow::Result<i64> {
    let content = format!("Lesson: {title} has a distinct ranking signal for context ordering.");
    save_lesson(
        conn,
        &SaveLessonRequest {
            session_id: Some("s1"),
            project: "/repo",
            topic_key: Some(title),
            title,
            content: &content,
            confidence,
            source_evidence: None,
            files: None,
            branch: None,
            scope: "project",
            created_at_epoch: None,
            stale_after_epoch: None,
        },
    )
}

fn set_lesson_rank(
    conn: &Connection,
    memory_id: i64,
    reinforcement_count: i64,
    last_reinforced_at_epoch: i64,
) {
    conn.execute(
        "UPDATE memory_lessons
         SET reinforcement_count = ?2, last_reinforced_at_epoch = ?3
         WHERE memory_id = ?1",
        params![memory_id, reinforcement_count, last_reinforced_at_epoch],
    )
    .unwrap();
}
