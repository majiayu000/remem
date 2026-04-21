use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};

use super::merge::MergeResult;

pub(super) fn apply(conn: &mut Connection, project: &str, result: &MergeResult) -> Result<()> {
    let tx = conn.transaction()?;

    // Upsert the merged memory (reuses existing topic_key upsert logic)
    crate::memory::insert_memory_full(
        &tx,
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

    for id in &result.superseded_ids {
        let updated = tx.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1 AND project = ?2",
            params![id, project],
        )?;
        if updated != 1 {
            return Err(anyhow!(
                "failed to mark superseded memory stale: id={} project={}",
                id,
                project
            ));
        }
    }

    tx.commit()?;
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

    fn active_count(conn: &Connection, project: &str, topic_key: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2 AND status = 'active'",
            params![project, topic_key],
            |row| row.get(0),
        )
        .expect("active count should query")
    }

    fn status_for_id(conn: &Connection, id: i64) -> String {
        conn.query_row(
            "SELECT status FROM memories WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .expect("status should query")
    }

    #[test]
    fn test_apply_upserts_merged_memory() {
        let (mut conn, project) = setup();
        let result = MergeResult {
            topic_key: "merged-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged title".to_owned(),
            content: "Merged content".to_owned(),
            superseded_ids: vec![],
        };
        apply(&mut conn, &project, &result).expect("apply");

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
        let (mut conn, project) = setup();
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
        apply(&mut conn, &project, &result).expect("apply");

        assert_eq!(status_for_id(&conn, old_id), "stale");
    }

    #[test]
    fn test_apply_rolls_back_when_stale_mark_update_fails() {
        let (mut conn, project) = setup();
        let old_id = insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            Some("old-topic"),
            "old title",
            "old content",
            "decision",
            None,
        )
        .expect("insert old memory");
        conn.execute_batch(
            "CREATE TRIGGER fail_stale_update
             BEFORE UPDATE OF status ON memories
             WHEN NEW.status = 'stale'
             BEGIN
                 SELECT RAISE(FAIL, 'forced stale update failure');
             END;",
        )
        .expect("trigger should install");

        let result = MergeResult {
            topic_key: "merged-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged title".to_owned(),
            content: "Merged content".to_owned(),
            superseded_ids: vec![old_id],
        };

        let error = apply(&mut conn, &project, &result).expect_err("apply should fail");
        assert!(
            error.to_string().contains("forced stale update failure"),
            "expected trigger failure, got: {error:?}"
        );

        assert_eq!(active_count(&conn, &project, "merged-topic"), 0);
        assert_eq!(status_for_id(&conn, old_id), "active");
    }

    #[test]
    fn test_apply_is_atomic_on_invalid_superseded_id() {
        // ID 99999 does not exist — stale-mark must fail, and the upsert must be rolled back.
        let (mut conn, project) = setup();
        let result = MergeResult {
            topic_key: "atomic-merged".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Atomic title".to_owned(),
            content: "Atomic content".to_owned(),
            superseded_ids: vec![99999],
        };
        assert!(
            apply(&mut conn, &project, &result).is_err(),
            "apply must fail when a superseded id does not exist"
        );

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2",
                params![project, "atomic-merged"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "upsert must be rolled back when stale-mark fails");
    }
}
