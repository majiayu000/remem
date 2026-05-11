use anyhow::{anyhow, Result};
use rusqlite::{params, Connection};

use super::merge::MergeResult;

pub(super) fn apply(conn: &mut Connection, project: &str, result: &MergeResult) -> Result<()> {
    let tx = conn.transaction()?;

    // Upsert the merged memory (reuses existing topic_key upsert logic)
    let merged_id = crate::memory::insert_memory_full(
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

    // Deduplicate before processing: a hallucinated duplicate id from the LLM
    // would otherwise re-fire the `memories_au` trigger on the second pass,
    // and the trigger's 'delete' against an already-removed FTS row can
    // surface as `database disk image is malformed` and abort the
    // transaction. `filter_superseded_ids` already drops out-of-cluster ids
    // but does not deduplicate.
    let mut seen = std::collections::HashSet::with_capacity(result.superseded_ids.len());
    let unique_ids: Vec<i64> = result
        .superseded_ids
        .iter()
        .copied()
        .filter(|id| *id != merged_id && seen.insert(*id))
        .collect();

    for id in &unique_ids {
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

    /// Whether the FTS index can return `id` via a MATCH on the term.
    /// For content='memories' fts5, rowid lookups proxy the source row,
    /// so MATCH is the authoritative probe for index membership.
    fn fts_indexed(conn: &Connection, id: i64, term: &str) -> bool {
        let mut stmt = conn
            .prepare(
                "SELECT 1 FROM memories_fts \
                 WHERE memories_fts MATCH ?1 AND rowid = ?2",
            )
            .expect("prepare fts probe");
        stmt.exists(params![term, id])
            .expect("fts probe should run")
    }

    #[test]
    fn test_apply_handles_duplicate_superseded_ids() {
        // Codex review: a hallucinated duplicate id from the LLM must not
        // re-fire the memories_au trigger on a row that has already been
        // removed from memories_fts (which can surface as "database disk
        // image is malformed").
        let (mut conn, project) = setup();
        let old_id = insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            None,
            "duplicateterm",
            "duplicate content",
            "decision",
            None,
        )
        .expect("insert");

        let result = MergeResult {
            topic_key: "dup-merged".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged title".to_owned(),
            content: "Merged content".to_owned(),
            superseded_ids: vec![old_id, old_id, old_id],
        };
        apply(&mut conn, &project, &result)
            .expect("apply must succeed even with duplicate superseded ids");

        assert_eq!(status_for_id(&conn, old_id), "stale");
        assert!(
            !fts_indexed(&conn, old_id, "duplicateterm"),
            "duplicated supersede must still leave the row out of memories_fts"
        );
    }

    #[test]
    fn test_apply_removes_superseded_from_fts_index() {
        let (mut conn, project) = setup();
        let old_id = insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            None,
            "uniqueoldtitle",
            "uniqueoldcontent",
            "decision",
            None,
        )
        .expect("insert");
        assert!(
            fts_indexed(&conn, old_id, "uniqueoldtitle"),
            "fresh insert must be indexed in memories_fts"
        );

        let result = MergeResult {
            topic_key: "fts-merged".to_owned(),
            memory_type: "decision".to_owned(),
            title: "New title".to_owned(),
            content: "New content".to_owned(),
            superseded_ids: vec![old_id],
        };
        apply(&mut conn, &project, &result).expect("apply");

        assert_eq!(status_for_id(&conn, old_id), "stale");
        assert!(
            !fts_indexed(&conn, old_id, "uniqueoldtitle"),
            "superseded memory must not match in memories_fts"
        );
    }

    #[test]
    fn test_apply_keeps_reused_topic_key_merge_active() {
        let (mut conn, project) = setup();
        let old_id = insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            Some("reused-topic"),
            "Old reused title",
            "oldreusedneedle content",
            "decision",
            None,
        )
        .expect("insert");

        let result = MergeResult {
            topic_key: "reused-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged reused title".to_owned(),
            content: "mergedreusedneedle content".to_owned(),
            superseded_ids: vec![old_id],
        };
        apply(&mut conn, &project, &result).expect("apply");

        assert_eq!(status_for_id(&conn, old_id), "active");
        assert_eq!(active_count(&conn, &project, "reused-topic"), 1);
        assert!(
            fts_indexed(&conn, old_id, "mergedreusedneedle"),
            "merged memory must remain searchable after topic_key reuse"
        );
        assert!(
            !fts_indexed(&conn, old_id, "oldreusedneedle"),
            "old content must not remain indexed after the upsert"
        );
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
    fn test_apply_evicts_superseded_rows_from_fts() {
        let (mut conn, project) = setup();
        let old_id = insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            None,
            "old searchable title",
            "supersededneedle older content",
            "decision",
            None,
        )
        .expect("insert old memory");

        let pre_hits: Vec<i64> = conn
            .prepare("SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1")
            .unwrap()
            .query_map(params!["supersededneedle"], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            pre_hits,
            vec![old_id],
            "FTS index should locate the original row before apply"
        );

        let result = MergeResult {
            topic_key: "merged-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged title".to_owned(),
            content: "Merged content".to_owned(),
            superseded_ids: vec![old_id],
        };
        apply(&mut conn, &project, &result).expect("apply");

        let post_hits: Vec<i64> = conn
            .prepare("SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1")
            .unwrap()
            .query_map(params!["supersededneedle"], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            post_hits.is_empty(),
            "FTS MATCH must not return superseded rows after apply, got: {post_hits:?}"
        );

        // The merged memory should still be searchable.
        let merged_hits: Vec<i64> = conn
            .prepare("SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1")
            .unwrap()
            .query_map(params!["Merged"], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            merged_hits.len(),
            1,
            "merged memory should remain indexed in FTS"
        );
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
