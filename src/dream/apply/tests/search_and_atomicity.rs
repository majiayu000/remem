use super::*;

#[test]
fn apply_hides_superseded_rows_through_query_predicate() {
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
    assert_eq!(
        post_hits,
        vec![old_id],
        "the all-status FTS index must retain the superseded row"
    );
    assert!(
        search_memories_fts(&conn, "supersededneedle", Some(&project), None, 10, 0)
            .expect("default search")
            .is_empty(),
        "the default query predicate must hide superseded rows"
    );
    let stale_hits = search_memories_fts_filtered(
        &conn,
        "supersededneedle",
        Some(&project),
        None,
        10,
        0,
        true,
        None,
    )
    .expect("include inactive search");
    assert_eq!(stale_hits.len(), 1);
    assert_eq!(stale_hits[0].id, old_id);
    assert_eq!(stale_hits[0].status, "stale");

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
fn apply_is_atomic_on_invalid_superseded_id() {
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

#[test]
fn apply_reused_topic_preserves_candidate_provenance_in_receipt() -> Result<()> {
    let (mut conn, project) = setup();
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (77, 'project', 'decision', 'reused-provenance',
                 'candidate source', '[501,502]', 0.9, 'low', 'approved', 1, 1)",
        [],
    )?;
    let old_id = insert_memory(
        &conn,
        Some("sess-1"),
        &project,
        Some("reused-provenance"),
        "Old title",
        "Old content",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET evidence_event_ids = '[501,502]', source_candidate_id = 77
         WHERE id = ?1",
        [old_id],
    )?;

    apply(
        &mut conn,
        &project,
        &MergeResult {
            topic_key: "reused-provenance".to_string(),
            memory_type: "decision".to_string(),
            title: "Consolidated title".to_string(),
            content: "Consolidated content".to_string(),
            superseded_ids: vec![old_id],
        },
    )?;

    let (evidence, candidate_id): (String, Option<i64>) = conn.query_row(
        "SELECT evidence_event_ids, source_candidate_id FROM memories WHERE id = ?1",
        [old_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(evidence, "[501,502]");
    assert_eq!(candidate_id, Some(77));
    let stored_result_sha256: String = conn.query_row(
        "SELECT result_sha256 FROM memory_activation_requests
         WHERE result_memory_id = ?1 AND route_kind = 'dream_consolidation'",
        [old_id],
        |row| row.get(0),
    )?;
    let actual = crate::memory::activation::ExpectedActiveMemory::from_existing(&conn, old_id)?;
    assert_eq!(stored_result_sha256, actual.sha256());
    Ok(())
}

#[test]
fn branch_scoped_topic_fails_closed_under_unscoped_dream_route() -> Result<()> {
    let (mut conn, project) = setup();
    let branch_id = crate::memory::insert_memory_with_branch(
        &conn,
        Some("sess-branch"),
        &project,
        Some("branch-scoped-topic"),
        "Branch-scoped title",
        "Branch-scoped content",
        "decision",
        None,
        Some("feature/branch"),
    )?;

    let error = apply(
        &mut conn,
        &project,
        &MergeResult {
            topic_key: "branch-scoped-topic".to_string(),
            memory_type: "decision".to_string(),
            title: "Unscoped consolidated title".to_string(),
            content: "Unscoped consolidated content".to_string(),
            superseded_ids: vec![branch_id],
        },
    )
    .expect_err("unscoped Dream must not supersede a branch-scoped source");
    assert_eq!(
        error.to_string(),
        format!("memory activation supersede target is missing, inactive, or outside route: {branch_id}")
    );
    assert_eq!(status_for_id(&conn, branch_id), "active");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        1,
        "only the Rust API receipt that seeded the branch row should remain"
    );
    Ok(())
}

#[test]
fn replay_returns_the_operation_bound_to_the_original_activation() -> Result<()> {
    let (mut conn, project) = setup();
    let memory_id = insert_memory(
        &conn,
        Some("dream-operation-replay"),
        &project,
        Some("dream-operation-replay"),
        "Original title",
        "Original content",
        "decision",
        None,
    )?;
    let first = MergeResult {
        topic_key: "dream-operation-replay".to_string(),
        memory_type: "decision".to_string(),
        title: "First consolidation".to_string(),
        content: "First consolidated value.".to_string(),
        superseded_ids: vec![memory_id],
    };
    let first_outcome = apply(&mut conn, &project, &first)?;
    let second = MergeResult {
        topic_key: "dream-operation-replay".to_string(),
        memory_type: "decision".to_string(),
        title: "Second consolidation".to_string(),
        content: "Second consolidated value.".to_string(),
        superseded_ids: vec![memory_id],
    };
    let second_outcome = apply(&mut conn, &project, &second)?;
    assert_eq!(first_outcome.merged_id, memory_id);
    assert_eq!(second_outcome.merged_id, memory_id);
    assert_ne!(first_outcome.operation_id, second_outcome.operation_id);

    let replay = apply(&mut conn, &project, &first)?;
    assert_eq!(replay.merged_id, memory_id);
    assert_eq!(replay.operation_id, first_outcome.operation_id);
    assert_ne!(replay.operation_id, second_outcome.operation_id);
    Ok(())
}

#[test]
fn replay_precedes_a_later_replacement_topic_collision() -> Result<()> {
    let (mut conn, project) = setup();
    let source_id = insert_memory(
        &conn,
        Some("dream-replaced-replay"),
        &project,
        Some("source-topic"),
        "Source title",
        "Source content",
        "decision",
        None,
    )?;
    let original = MergeResult {
        topic_key: "stable-dream-topic".to_string(),
        memory_type: "decision".to_string(),
        title: "Original consolidation".to_string(),
        content: "Original consolidated value.".to_string(),
        superseded_ids: vec![source_id],
    };
    let original_outcome = apply(&mut conn, &project, &original)?;

    let replacement = crate::memory::lifecycle::apply_update(
        &conn,
        Some("later-governed-replacement"),
        &project,
        "stable-dream-topic",
        "Later replacement",
        "Later governed value.",
        "decision",
        None,
        None,
        "project",
        &[original_outcome.merged_id],
    )?;
    let replacement_id = replacement.memory_id.expect("replacement id");
    assert_ne!(replacement_id, original_outcome.merged_id);

    let replay = apply(&mut conn, &project, &original)?;
    assert_eq!(replay.merged_id, original_outcome.merged_id);
    assert_eq!(replay.operation_id, original_outcome.operation_id);
    assert_eq!(status_for_id(&conn, original_outcome.merged_id), "stale");
    assert_eq!(status_for_id(&conn, replacement_id), "active");
    Ok(())
}

#[test]
fn replay_uses_receipt_identity_after_same_row_provenance_is_cleared() -> Result<()> {
    let (mut conn, project) = setup();
    conn.execute(
        "INSERT INTO memory_candidates
         (id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (88, 'project', 'decision', 'dream-provenance-replay',
                 'candidate source', '[601]', 0.9, 'low', 'approved', 1, 1)",
        [],
    )?;
    let memory_id = insert_memory(
        &conn,
        Some("dream-provenance-replay"),
        &project,
        Some("dream-provenance-replay"),
        "Seed title",
        "Seed content",
        "decision",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET evidence_event_ids = '[601]', source_candidate_id = 88
         WHERE id = ?1",
        [memory_id],
    )?;
    let original = MergeResult {
        topic_key: "dream-provenance-replay".to_string(),
        memory_type: "decision".to_string(),
        title: "Dream consolidated title".to_string(),
        content: "Dream consolidated value.".to_string(),
        superseded_ids: vec![memory_id],
    };
    let original_outcome = apply(&mut conn, &project, &original)?;

    let later_id = crate::memory::insert_memory_full(
        &conn,
        Some("later-direct-update"),
        &project,
        Some("dream-provenance-replay"),
        "Later direct title",
        "Later direct value.",
        "decision",
        None,
        None,
        "project",
        None,
    )?;
    assert_eq!(later_id, memory_id, "later update must reuse the Dream row");
    let candidate_id: Option<i64> = conn.query_row(
        "SELECT source_candidate_id FROM memories WHERE id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    assert_eq!(candidate_id, None);

    let replay = apply(&mut conn, &project, &original)?;
    assert_eq!(replay.merged_id, original_outcome.merged_id);
    assert_eq!(replay.operation_id, original_outcome.operation_id);
    Ok(())
}
