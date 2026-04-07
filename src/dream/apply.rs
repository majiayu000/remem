use anyhow::Result;
use rusqlite::{params, Connection};

use super::merge::MergeResult;

pub(super) fn apply(conn: &Connection, project: &str, result: &MergeResult) -> Result<()> {
    // Upsert the merged memory (reuses existing topic_key upsert logic)
    crate::memory::insert_memory_full(
        conn,
        Some("dream"),
        project,
        Some(&result.topic_key),
        &result.title,
        &result.content,
        &result.memory_type,
        None,
        None,
        "project",
        None,
    )?;

    // Mark superseded memories as stale
    for id in &result.superseded_ids {
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1 AND project = ?2",
            params![id, project],
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::insert_memory;
    use crate::memory::tests_helper::setup_memory_schema;
    use rusqlite::Connection;

    fn setup() -> (Connection, String) {
        let conn = Connection::open_in_memory().expect("in-memory db");
        setup_memory_schema(&conn);
        let project = "test-dream-apply".to_owned();
        (conn, project)
    }

    #[test]
    fn test_apply_upserts_merged_memory() {
        let (conn, project) = setup();
        let result = MergeResult {
            topic_key: "merged-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged title".to_owned(),
            content: "Merged content".to_owned(),
            superseded_ids: vec![],
        };
        apply(&conn, &project, &result).expect("apply");

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2",
                params![project, "merged-topic"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_apply_marks_superseded_stale() {
        let (conn, project) = setup();
        let old_id = insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            None,
            "old title",
            "old content",
            "decision",
            None,
        )
        .expect("insert");

        let result = MergeResult {
            topic_key: "new-merged".to_owned(),
            memory_type: "decision".to_owned(),
            title: "New title".to_owned(),
            content: "New content".to_owned(),
            superseded_ids: vec![old_id],
        };
        apply(&conn, &project, &result).expect("apply");

        let status: String = conn
            .query_row(
                "SELECT status FROM memories WHERE id = ?1",
                params![old_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "stale");
    }
}
