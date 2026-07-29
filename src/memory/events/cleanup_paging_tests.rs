use rusqlite::{params, Connection};

use super::tests::{
    insert_observation_row, link_source, observation, observation_exists,
    setup_observation_retention_schema,
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
