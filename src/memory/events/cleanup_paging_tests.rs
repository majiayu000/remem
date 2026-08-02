use rusqlite::{params, Connection};

use super::tests::{
    insert_observation_row, link_source, observation, observation_exists,
    rewrite_source_link_as_v1, setup_observation_retention_schema,
};
use super::{
    cleanup_compressed_source_observations_at, count_compressed_source_observations_to_delete_at,
    COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
};

#[test]
fn compressed_source_cleanup_crosses_scan_batch_boundary() {
    let conn = Connection::open_in_memory().expect("in-memory database should open");
    setup_observation_retention_schema(&conn);
    let now = 2_000_000_000;
    let old_epoch = now - 400 * 86_400;
    let old_link_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400;
    let replacement = observation(10_000, "active", old_epoch, "replacement");
    insert_observation_row(&conn, &replacement);

    for id in 1..=501 {
        let source = observation(id, "compressed", old_epoch, &format!("source-{id}"));
        insert_observation_row(&conn, &source);
        link_source(&conn, replacement.id, &source, old_link_epoch);
    }

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("cleanup plan should scan every page"),
        501
    );
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("cleanup apply should scan every page"),
        501
    );
    for id in [1, 500, 501] {
        assert!(
            !observation_exists(&conn, id),
            "source {id} should be deleted"
        );
    }
    assert!(observation_exists(&conn, replacement.id));
}

#[test]
fn v2_snapshot_allows_cleanup_with_complete_modern_provenance() {
    let conn = Connection::open_in_memory().expect("in-memory database should open");
    setup_observation_retention_schema(&conn);
    let now = 2_000_000_000;
    let old_epoch = now - 400 * 86_400;
    let old_link_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400;
    let replacement = observation(100, "active", old_epoch, "replacement");
    let source = observation(1, "compressed", old_epoch, "modern source");
    insert_observation_row(&conn, &replacement);
    insert_observation_row(&conn, &source);
    conn.execute(
        "UPDATE observations
         SET prompt_number = 4, last_accessed_epoch = ?1,
             host_id = 1, project_id = 2, session_row_id = 3,
             observation_type = 'decision', text = 'complete evidence',
             evidence_event_ids = '[7]', confidence = 0.9,
             reference_time_epoch = ?2
         WHERE id = ?3",
        params![old_epoch + 1, old_epoch, source.id],
    )
    .expect("modern provenance should be populated before snapshot");
    link_source(&conn, replacement.id, &source, old_link_epoch);

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("v2 plan should accept complete modern provenance"),
        1
    );
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("v2 apply should accept complete modern provenance"),
        1
    );
    assert!(!observation_exists(&conn, source.id));
    assert!(observation_exists(&conn, replacement.id));
}

#[test]
fn malformed_legacy_link_does_not_hide_a_valid_v2_link() {
    let conn = Connection::open_in_memory().expect("in-memory database should open");
    setup_observation_retention_schema(&conn);
    let now = 2_000_000_000;
    let old_epoch = now - 400 * 86_400;
    let old_link_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400;
    let invalid_replacement = observation(100, "active", old_epoch, "invalid replacement");
    let valid_replacement = observation(101, "active", old_epoch, "valid replacement");
    let source = observation(1, "compressed", old_epoch, "source");
    for item in [&invalid_replacement, &valid_replacement, &source] {
        insert_observation_row(&conn, item);
    }
    link_source(&conn, invalid_replacement.id, &source, old_link_epoch);
    link_source(&conn, valid_replacement.id, &source, old_link_epoch);
    conn.execute(
        "UPDATE compressed_observation_sources
         SET source_hash = ?1, source_snapshot_json = '{'
         WHERE compressed_observation_id = ?2 AND source_observation_id = ?3",
        params![
            crate::db::observation_source_hash(&source),
            invalid_replacement.id,
            source.id
        ],
    )
    .expect("legacy link should be corrupted for the regression fixture");

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("valid v2 provenance should remain sufficient"),
        1
    );
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("invalid alternate provenance should not block cleanup"),
        1
    );
    assert!(!observation_exists(&conn, source.id));
}

#[test]
fn access_and_joined_session_changes_do_not_invalidate_v2_provenance() {
    let conn = Connection::open_in_memory().expect("in-memory database should open");
    setup_observation_retention_schema(&conn);
    let now = 2_000_000_000;
    let old_epoch = now - 400 * 86_400;
    let old_link_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400;
    let replacement = observation(100, "active", old_epoch, "replacement");
    let source = observation(1, "compressed", old_epoch, "source");
    insert_observation_row(&conn, &replacement);
    insert_observation_row(&conn, &source);
    link_source(&conn, replacement.id, &source, old_link_epoch);

    conn.execute(
        "UPDATE observations SET last_accessed_epoch = ?1 WHERE id = ?2",
        params![old_epoch + 10, source.id],
    )
    .expect("access feedback should update");
    for content_session_id in ["content-a", "content-b"] {
        conn.execute(
            "INSERT INTO sdk_sessions
             (content_session_id, memory_session_id, project)
             VALUES (?1, ?2, 'proj')",
            params![content_session_id, source.memory_session_id],
        )
        .expect("joined session context should insert");
    }

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("mutable access and joined context must not change v2 provenance"),
        1
    );
    conn.execute(
        "DELETE FROM sdk_sessions WHERE content_session_id = 'content-a'",
        [],
    )
    .expect("joined session context should change independently");
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("joined context changes must remain noncanonical"),
        1
    );
    assert!(!observation_exists(&conn, source.id));
}

#[test]
fn malformed_v2_snapshot_preserves_the_source() {
    let conn = Connection::open_in_memory().expect("in-memory database should open");
    setup_observation_retention_schema(&conn);
    let now = 2_000_000_000;
    let old_epoch = now - 400 * 86_400;
    let old_link_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400;
    let replacement = observation(100, "active", old_epoch, "replacement");
    let source = observation(1, "compressed", old_epoch, "source");
    insert_observation_row(&conn, &replacement);
    insert_observation_row(&conn, &source);
    link_source(&conn, replacement.id, &source, old_link_epoch);
    conn.execute(
        "UPDATE compressed_observation_sources
         SET source_snapshot_json = '{'
         WHERE source_observation_id = ?1",
        params![source.id],
    )
    .expect("v2 snapshot should be corrupted for the regression fixture");

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("malformed v2 provenance should fail closed for this source"),
        0
    );
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("malformed v2 provenance should not abort unrelated cleanup"),
        0
    );
    assert!(observation_exists(&conn, source.id));
}

#[test]
fn exact_legacy_links_upgrade_to_v2_before_historical_sources_are_deleted() {
    let conn = Connection::open_in_memory().expect("in-memory database should open");
    setup_observation_retention_schema(&conn);
    let now = 2_000_000_000;
    let old_epoch = now - 400 * 86_400;
    let old_link_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400;
    let replacement = observation(100, "active", old_epoch, "replacement");
    insert_observation_row(&conn, &replacement);
    let sources = (1..=5)
        .map(|id| observation(id, "compressed", old_epoch, &format!("source-{id}")))
        .collect::<Vec<_>>();
    for source in &sources {
        insert_observation_row(&conn, source);
        link_source(&conn, replacement.id, source, old_link_epoch);
        rewrite_source_link_as_v1(&conn, source);
    }
    conn.execute("UPDATE observations SET prompt_number = 7 WHERE id = 1", [])
        .expect("prompt provenance should update");
    conn.execute(
        "UPDATE observations SET last_accessed_epoch = ?1 WHERE id = 2",
        params![old_epoch + 1],
    )
    .expect("access feedback should update");
    conn.execute(
        "UPDATE observations SET observation_type = 'decision' WHERE id = 3",
        [],
    )
    .expect("typed provenance should update");
    conn.execute(
        "UPDATE observations SET text = 'unique evidence' WHERE id = 4",
        [],
    )
    .expect("evidence text should update");
    conn.execute(
        "UPDATE observations SET reference_time_epoch = created_at_epoch WHERE id = 5",
        [],
    )
    .expect("reference-time provenance should update");

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("exact v1 links should be safely upgradable"),
        5
    );
    let v1_links_before_apply: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM compressed_observation_sources
             WHERE source_hash LIKE 'sha256:observation-v1:%'",
            [],
            |row| row.get(0),
        )
        .expect("preview should remain read-only");
    assert_eq!(v1_links_before_apply, 5);

    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
        )
        .expect("apply should upgrade v1 provenance before deletion"),
        5
    );
    for source in &sources {
        assert!(!observation_exists(&conn, source.id));
    }
    let upgraded_links = conn
        .prepare(
            "SELECT source_observation_id, source_hash, source_snapshot_json
             FROM compressed_observation_sources
             ORDER BY source_observation_id",
        )
        .expect("upgraded links should query")
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .expect("upgraded links should map")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("upgraded links should collect");
    assert_eq!(upgraded_links.len(), 5);
    for (source_id, source_hash, snapshot_json) in upgraded_links {
        assert!(source_hash.starts_with("sha256:observation-v2:"));
        let snapshot: serde_json::Value =
            serde_json::from_str(&snapshot_json).expect("v2 snapshot should remain valid JSON");
        assert_eq!(snapshot["hash_version"], "observation-v2");
        assert_eq!(snapshot["id"], source_id);
    }
}
