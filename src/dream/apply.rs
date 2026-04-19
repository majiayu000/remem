use anyhow::Result;
use rusqlite::{params, Connection};

use super::merge::MergeResult;

pub(super) fn apply(conn: &mut Connection, project: &str, result: &MergeResult) -> Result<()> {
    let tx = conn.transaction()?;

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
        tx.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1 AND project = ?2",
            params![id, project],
        )?;
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

        let status: String = conn
            .query_row(
                "SELECT status FROM memories WHERE id = ?1",
                params![old_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "stale");
    }

    /// Proves the all-or-nothing guarantee: if the stale-mark UPDATE fails after
    /// `insert_memory_full` has already run, the entire transaction must roll back
    /// so no partial state is persisted.
    #[test]
    fn test_apply_rolls_back_insert_when_stale_mark_fails() {
        let (mut conn, project) = setup();

        // Trigger causes every stale-mark UPDATE to fail, simulating a mid-transaction crash.
        conn.execute_batch(
            "CREATE TRIGGER force_stale_fail
             BEFORE UPDATE OF status ON memories
             WHEN NEW.status = 'stale'
             BEGIN
                 SELECT RAISE(ABORT, 'forced stale-mark failure');
             END;",
        )
        .expect("create trigger");

        let old_id = insert_memory(
            &conn,
            Some("sess-x"),
            &project,
            None,
            "predecessor",
            "will be superseded",
            "decision",
            None,
        )
        .expect("insert predecessor");

        let result = MergeResult {
            topic_key: "rollback-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Should not persist".to_owned(),
            content: "Transaction must roll back".to_owned(),
            superseded_ids: vec![old_id],
        };

        assert!(
            apply(&mut conn, &project, &result).is_err(),
            "apply() must propagate the stale-mark failure"
        );

        // The merged memory must NOT be present — insert was rolled back.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = ?2",
                params![&project, "rollback-topic"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "merged memory insert must be rolled back");

        // Predecessor must still be active — stale-mark was also rolled back.
        let status: String = conn
            .query_row(
                "SELECT status FROM memories WHERE id = ?1",
                params![old_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "active", "predecessor status must be unchanged on rollback");
    }
}
