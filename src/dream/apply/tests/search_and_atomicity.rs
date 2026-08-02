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
