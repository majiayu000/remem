use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::merge::MergeResult;
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::operation::{insert_operation_log, MemoryOperationInput, MemoryOperationPlan};

pub(super) fn apply(conn: &mut Connection, project: &str, result: &MergeResult) -> Result<()> {
    let tx = conn.transaction()?;
    let superseded_ids =
        validate_dream_superseded_ids(&tx, project, &result.memory_type, &result.superseded_ids)?;
    validate_dream_target_topic(
        &tx,
        project,
        &result.memory_type,
        &result.topic_key,
        &superseded_ids,
    )?;
    let state_key = crate::memory::state_key::derive_state_key(
        &result.memory_type,
        Some(&result.topic_key),
        &result.title,
        &result.content,
    )
    .map(|decision| decision.state_key);
    let operation_input = MemoryOperationInput {
        source: "dream".to_string(),
        actor: "dream".to_string(),
        source_project: project.to_string(),
        owner_scope: "repo".to_string(),
        owner_key: project.to_string(),
        memory_type: result.memory_type.clone(),
        topic_key: Some(result.topic_key.clone()),
        state_key: state_key.clone(),
        source_candidate_id: None,
        confidence: None,
    };

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
    let actual_superseded_ids = superseded_ids
        .into_iter()
        .filter(|id| *id != merged_id)
        .collect::<Vec<_>>();

    crate::memory::lifecycle::soft_supersede(
        &tx,
        project,
        &actual_superseded_ids,
        Some(merged_id),
    )?;
    let op = if result.superseded_ids.is_empty() {
        MemoryLifecycleOp::Add
    } else {
        MemoryLifecycleOp::Update
    };
    let plan = MemoryOperationPlan::new(op, state_key, "dream consolidation applied")
        .with_target_memory_id(Some(merged_id))
        .with_superseded_ids(actual_superseded_ids);
    insert_operation_log(&tx, &operation_input, &plan, Some(merged_id))?;

    tx.commit()?;
    Ok(())
}

fn validate_dream_superseded_ids(
    conn: &Connection,
    project: &str,
    memory_type: &str,
    superseded_ids: &[i64],
) -> Result<Vec<i64>> {
    let mut seen = std::collections::HashSet::with_capacity(superseded_ids.len());
    let mut valid = Vec::new();
    for id in superseded_ids.iter().copied().filter(|id| seen.insert(*id)) {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM memories
                 WHERE id = ?1
                   AND project = ?2
                   AND memory_type = ?3
                   AND COALESCE(
                        owner_scope,
                        CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
                   ) = 'repo'
                   AND COALESCE(
                        owner_key,
                        CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END
                   ) = ?2
             )",
            params![id, project, memory_type],
            |row| row.get(0),
        )?;
        if !exists {
            bail!("dream superseded memory id={id} is outside project/type/owner neighborhood");
        }
        valid.push(id);
    }
    Ok(valid)
}

fn validate_dream_target_topic(
    conn: &Connection,
    project: &str,
    memory_type: &str,
    topic_key: &str,
    superseded_ids: &[i64],
) -> Result<()> {
    let existing_id = conn
        .query_row(
            "SELECT id FROM memories
             WHERE project = ?1
               AND memory_type = ?2
               AND topic_key = ?3
               AND COALESCE(
                    owner_scope,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END
               ) = 'repo'
               AND COALESCE(
                    owner_key,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END
               ) = ?1
             ORDER BY CASE status WHEN 'active' THEN 0 ELSE 1 END,
                      updated_at_epoch DESC,
                      id DESC
             LIMIT 1",
            params![project, memory_type, topic_key],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(existing_id) = existing_id else {
        return Ok(());
    };
    if superseded_ids.contains(&existing_id) {
        return Ok(());
    }
    bail!(
        "dream target topic_key collides with memory id={existing_id} outside superseded neighborhood"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::insert_memory;
    use crate::memory::tests_helper::setup_memory_schema;
    use rusqlite::{params, Connection};

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
    fn test_apply_keeps_reused_topic_key_merge_active() -> Result<()> {
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
        )?;

        let result = MergeResult {
            topic_key: "reused-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged reused title".to_owned(),
            content: "mergedreusedneedle content".to_owned(),
            superseded_ids: vec![old_id],
        };
        apply(&mut conn, &project, &result)?;

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
        let operation: String = conn.query_row(
            "SELECT operation FROM memory_operation_log ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(operation, "update");
        Ok(())
    }

    #[test]
    fn test_apply_rejects_target_topic_collision_outside_superseded_neighborhood() -> Result<()> {
        let (mut conn, project) = setup();
        insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            Some("collision-topic"),
            "Unrelated title",
            "unrelated content",
            "decision",
            None,
        )?;
        let old_id = insert_memory(
            &conn,
            Some("sess-1"),
            &project,
            Some("old-topic"),
            "Old title",
            "old content",
            "decision",
            None,
        )?;

        let result = MergeResult {
            topic_key: "collision-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged title".to_owned(),
            content: "merged content".to_owned(),
            superseded_ids: vec![old_id],
        };

        let error = apply(&mut conn, &project, &result).expect_err("collision should fail");
        assert!(
            error.to_string().contains("target topic_key collides"),
            "expected collision error, got: {error:?}"
        );
        assert_eq!(status_for_id(&conn, old_id), "active");
        let log_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_operation_log", [], |row| {
                row.get(0)
            })?;
        assert_eq!(log_count, 0);
        Ok(())
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
    fn test_apply_records_operation_log_for_superseded_ids() -> Result<()> {
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
        )?;

        let result = MergeResult {
            topic_key: "logged-merged".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Logged title".to_owned(),
            content: "Logged content".to_owned(),
            superseded_ids: vec![old_id],
        };
        apply(&mut conn, &project, &result)?;

        let (operation, result_memory_id, superseded_ids): (String, i64, String) = conn.query_row(
            "SELECT operation, result_memory_id, superseded_ids
             FROM memory_operation_log
             ORDER BY id DESC
             LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(operation, "update");
        assert_ne!(result_memory_id, old_id);
        assert_eq!(
            serde_json::from_str::<Vec<i64>>(&superseded_ids)?,
            vec![old_id]
        );
        Ok(())
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
    fn test_apply_rolls_back_when_operation_log_insert_fails() -> Result<()> {
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
        )?;
        conn.execute_batch(
            "CREATE TRIGGER fail_operation_log_insert
             BEFORE INSERT ON memory_operation_log
             BEGIN
                 SELECT RAISE(FAIL, 'forced operation log failure');
             END;",
        )?;

        let result = MergeResult {
            topic_key: "merged-topic".to_owned(),
            memory_type: "decision".to_owned(),
            title: "Merged title".to_owned(),
            content: "Merged content".to_owned(),
            superseded_ids: vec![old_id],
        };

        let error = apply(&mut conn, &project, &result).expect_err("apply should fail");
        let error_chain = format!("{error:?}");
        assert!(
            error_chain.contains("forced operation log failure"),
            "expected operation log trigger failure, got: {error_chain}"
        );

        let log_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_operation_log", [], |row| {
                row.get(0)
            })?;
        assert_eq!(active_count(&conn, &project, "merged-topic"), 0);
        assert_eq!(status_for_id(&conn, old_id), "active");
        assert_eq!(log_count, 0);
        Ok(())
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
