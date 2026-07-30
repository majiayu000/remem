use rusqlite::{params, Connection};

use super::{
    archive_stale_memories, cleanup_compressed_source_observations_at, cleanup_old_events,
    count_compressed_source_observations_to_delete_at, count_old_events_at, get_session_events,
    get_session_files_modified, insert_event, COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
};
use crate::db::Observation;
use crate::memory::tests_helper::setup_memory_schema;

#[test]
fn test_event_insert_and_query() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    insert_event(
        &conn,
        "session-1",
        "proj",
        "file_edit",
        "Edit src/db.rs",
        None,
        Some(r#"["src/db.rs"]"#),
        None,
    )
    .unwrap();
    insert_event(
        &conn,
        "session-1",
        "proj",
        "bash",
        "Run `cargo test` (exit 0)",
        None,
        None,
        Some(0),
    )
    .unwrap();

    let events = get_session_events(&conn, "session-1").unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "file_edit");
    assert_eq!(events[1].exit_code, Some(0));
    let retention_classes = conn
        .prepare("SELECT retention_class FROM events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(retention_classes, vec!["ephemeral", "ephemeral"]);
}

#[test]
fn test_get_session_files_modified_dedups_entries() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    insert_event(
        &conn,
        "session-1",
        "proj",
        "file_edit",
        "Edit sources",
        None,
        Some(r#"["src/lib.rs","src/main.rs"]"#),
        None,
    )
    .unwrap();
    insert_event(
        &conn,
        "session-1",
        "proj",
        "file_create",
        "Create main",
        None,
        Some(r#"["src/main.rs","src/bin.rs"]"#),
        None,
    )
    .unwrap();
    insert_event(
        &conn,
        "session-1",
        "proj",
        "bash",
        "Run tests",
        None,
        Some("not-json"),
        Some(0),
    )
    .unwrap();

    let files = get_session_files_modified(&conn, "session-1").unwrap();
    assert_eq!(files, vec!["src/lib.rs", "src/main.rs", "src/bin.rs"]);
}

#[test]
fn test_cleanup_old_events() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let now = chrono::Utc::now().timestamp();
    let old = now - (31 * 86400);
    conn.execute(
        "INSERT INTO events
         (session_id, project, event_type, summary, created_at_epoch, retention_class)
         VALUES ('s1', 'proj', 'file_edit', 'old edit', ?1, 'ephemeral')",
        params![old],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO events (session_id, project, event_type, summary, created_at_epoch)
         VALUES ('s2', 'proj', 'file_edit', 'new edit', ?1)",
        params![now],
    )
    .unwrap();

    assert_eq!(cleanup_old_events(&conn, 30).unwrap(), 1);
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .unwrap();
    assert_eq!(remaining, 1);
}

#[test]
fn old_event_cleanup_fails_closed_without_retention_metadata() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE events (
           id INTEGER PRIMARY KEY,
           session_id TEXT NOT NULL,
           project TEXT NOT NULL,
           event_type TEXT NOT NULL,
           summary TEXT NOT NULL,
           created_at_epoch INTEGER NOT NULL
         );",
    )
    .unwrap();
    let now = 2_000_000_000;
    conn.execute(
        "INSERT INTO events (session_id, project, event_type, summary, created_at_epoch)
         VALUES ('s1', 'proj', 'file_edit', 'old edit', ?1)",
        params![now - 40 * 86_400],
    )
    .unwrap();

    assert_eq!(count_old_events_at(&conn, now, 30).unwrap(), 0);
    assert_eq!(super::cleanup_old_events_at(&conn, now, 30).unwrap(), 0);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM events", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn old_event_cleanup_preserves_audit_and_api_provenance() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);
    conn.execute_batch(
        "CREATE TABLE api_mutation_requests (
           idempotency_key_hash TEXT PRIMARY KEY,
           audit_id INTEGER NOT NULL
         );",
    )
    .unwrap();
    let now = 2_000_000_000;
    for (id, retention_class) in [(1_i64, "ephemeral"), (2, "audit"), (3, "ephemeral")] {
        conn.execute(
            "INSERT INTO events
             (id, session_id, project, event_type, summary, created_at_epoch, retention_class)
             VALUES (?1, 's', 'proj', 'file_edit', 'old', ?2, ?3)",
            params![id, now - 40 * 86_400, retention_class],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO api_mutation_requests (idempotency_key_hash, audit_id)
         VALUES ('request', 3)",
        [],
    )
    .unwrap();

    assert_eq!(super::cleanup_old_events_at(&conn, now, 30).unwrap(), 1);
    let remaining = conn
        .prepare("SELECT id FROM events ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, i64>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(remaining, vec![2, 3]);
}

#[test]
fn test_archive_stale_memories() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let now = chrono::Utc::now().timestamp();
    let old = now - (181 * 86400);
    conn.execute(
        "INSERT INTO memories (session_id, project, title, content, memory_type, \
         created_at_epoch, updated_at_epoch, status)
         VALUES ('s1', 'proj', 'old', 'old content', 'decision', ?1, ?1, 'stale')",
        params![old],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories (session_id, project, title, content, memory_type, \
         created_at_epoch, updated_at_epoch, status)
         VALUES ('s2', 'proj', 'new', 'new content', 'decision', ?1, ?1, 'active')",
        params![now],
    )
    .unwrap();

    assert_eq!(archive_stale_memories(&conn, 180).unwrap(), 1);
    let archived: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'archived'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(archived, 1);
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, 1);
}

#[test]
fn test_archive_stale_memories_does_not_archive_old_active_durable_memory() {
    let conn = Connection::open_in_memory().unwrap();
    setup_memory_schema(&conn);

    let old = chrono::Utc::now().timestamp() - (365 * 86400);
    conn.execute(
        "INSERT INTO memories (session_id, project, title, content, memory_type, \
         created_at_epoch, updated_at_epoch, status)
         VALUES ('s1', 'proj', 'old decision', 'keep durable decision', 'architecture', ?1, ?1, 'active')",
        params![old],
    )
    .unwrap();

    assert_eq!(archive_stale_memories(&conn, 180).unwrap(), 0);
    let status: String = conn
        .query_row("SELECT status FROM memories", [], |row| row.get(0))
        .unwrap();
    assert_eq!(status, "active");
}

#[test]
fn compressed_source_cleanup_only_deletes_old_sources_with_sufficient_provenance() {
    let conn = Connection::open_in_memory().unwrap();
    setup_observation_retention_schema(&conn);

    let now = 2_000_000_000;
    let old_epoch = now - (400 * 86_400);
    let old_link_epoch = now - ((COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400);
    let cutoff_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS * 86_400);
    let recent_link_epoch = now - ((COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS - 1) * 86_400);

    let replacement = observation(100, "active", old_epoch, "replacement");
    insert_observation_row(&conn, &replacement);

    let active = observation(1, "active", old_epoch, "active");
    let stale = observation(2, "stale", old_epoch, "stale");
    let eligible = observation_with_content_session(
        3,
        "compressed",
        old_epoch,
        "eligible",
        "content-session-eligible",
    );
    let recent = observation(4, "compressed", old_epoch, "recent");
    let missing_provenance = observation(5, "compressed", old_epoch, "missing provenance");
    let boundary = observation(8, "compressed", old_epoch, "boundary");
    for source in [
        &active,
        &stale,
        &eligible,
        &recent,
        &missing_provenance,
        &boundary,
    ] {
        insert_observation_row(&conn, source);
    }
    link_source(&conn, replacement.id, &eligible, old_link_epoch);
    link_source(&conn, replacement.id, &recent, recent_link_epoch);
    link_source(&conn, replacement.id, &boundary, cutoff_epoch);

    let nested_replacement = observation(6, "compressed", old_epoch, "nested replacement");
    let nested_source = observation(7, "active", old_epoch, "nested source");
    insert_observation_row(&conn, &nested_replacement);
    insert_observation_row(&conn, &nested_source);
    link_source(&conn, nested_replacement.id, &nested_source, old_link_epoch);
    link_source(&conn, replacement.id, &nested_replacement, old_link_epoch);

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS
        )
        .unwrap(),
        1
    );
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS
        )
        .unwrap(),
        1
    );

    assert!(!observation_exists(&conn, eligible.id));
    for id in [
        active.id,
        stale.id,
        recent.id,
        missing_provenance.id,
        nested_replacement.id,
        nested_source.id,
        boundary.id,
        replacement.id,
    ] {
        assert!(
            observation_exists(&conn, id),
            "observation {id} should remain"
        );
    }
    assert_eq!(source_link_count(&conn, eligible.id), 1);
}

#[test]
fn compressed_source_cleanup_blocks_hash_mismatch() {
    let conn = Connection::open_in_memory().unwrap();
    setup_observation_retention_schema(&conn);

    let now = 2_000_000_000;
    let old_epoch = now - (400 * 86_400);
    let old_link_epoch = now - ((COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400);

    let replacement = observation(100, "active", old_epoch, "replacement");
    let source = observation(1, "compressed", old_epoch, "source");
    insert_observation_row(&conn, &replacement);
    insert_observation_row(&conn, &source);
    link_source(&conn, replacement.id, &source, old_link_epoch);
    conn.execute(
        "UPDATE compressed_observation_sources
         SET source_hash = 'sha256:observation-v1:bad'
         WHERE source_observation_id = ?1",
        params![source.id],
    )
    .unwrap();

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS
        )
        .unwrap(),
        0
    );
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS
        )
        .unwrap(),
        0
    );
    assert!(observation_exists(&conn, source.id));

    conn.execute(
        "UPDATE compressed_observation_sources
         SET source_hash = ?1, source_snapshot_json = '{}'
         WHERE source_observation_id = ?2",
        params![crate::db::observation_source_hash(&source), source.id],
    )
    .unwrap();
    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS
        )
        .unwrap(),
        0
    );
    assert!(observation_exists(&conn, source.id));
}

#[test]
fn compressed_source_cleanup_preserves_incomplete_or_still_referenced_sources() {
    let conn = Connection::open_in_memory().unwrap();
    setup_observation_retention_schema(&conn);
    conn.execute_batch(
        "CREATE TABLE memory_facts (
           id INTEGER PRIMARY KEY,
           source_observation_id INTEGER
         );",
    )
    .unwrap();

    let now = 2_000_000_000;
    let old_epoch = now - 400 * 86_400;
    let old_link_epoch = now - (COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS + 1) * 86_400;

    let stale_replacement = observation(100, "stale", old_epoch, "stale replacement");
    let missing_replacement = observation(101, "active", old_epoch, "missing replacement");
    let active_replacement_a = observation(102, "active", old_epoch, "active replacement a");
    let active_replacement_b = observation(103, "active", old_epoch, "active replacement b");
    let stale_source = observation(1, "compressed", old_epoch, "stale replacement source");
    let missing_source = observation(2, "compressed", old_epoch, "missing replacement source");
    let extra_source = observation(3, "compressed", old_epoch, "extra provenance source");
    let fact_source = observation(4, "compressed", old_epoch, "fact referenced source");
    for observation in [
        &stale_replacement,
        &missing_replacement,
        &active_replacement_a,
        &active_replacement_b,
        &stale_source,
        &missing_source,
        &extra_source,
        &fact_source,
    ] {
        insert_observation_row(&conn, observation);
    }
    link_source(&conn, stale_replacement.id, &stale_source, old_link_epoch);
    link_source(
        &conn,
        missing_replacement.id,
        &missing_source,
        old_link_epoch,
    );
    link_source(
        &conn,
        active_replacement_a.id,
        &extra_source,
        old_link_epoch,
    );
    link_source(&conn, active_replacement_b.id, &fact_source, old_link_epoch);
    conn.execute(
        "DELETE FROM observations WHERE id = ?1",
        params![missing_replacement.id],
    )
    .unwrap();
    conn.execute(
        "UPDATE observations SET confidence = 0.9 WHERE id = ?1",
        params![extra_source.id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_facts (id, source_observation_id) VALUES (1, ?1)",
        params![fact_source.id],
    )
    .unwrap();

    assert_eq!(
        count_compressed_source_observations_to_delete_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS
        )
        .unwrap(),
        0
    );
    assert_eq!(
        cleanup_compressed_source_observations_at(
            &conn,
            now,
            COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS
        )
        .unwrap(),
        0
    );
    for id in [
        stale_source.id,
        missing_source.id,
        extra_source.id,
        fact_source.id,
    ] {
        assert!(observation_exists(&conn, id));
    }

    conn.execute_batch("ALTER TABLE observations ADD COLUMN future_provenance TEXT;")
        .unwrap();
    let producer_error = crate::db::observation_source_retention_record(&conn, &extra_source)
        .expect_err("new provenance columns must also block snapshot production");
    assert!(
        producer_error.to_string().contains("future_provenance"),
        "{producer_error:#}"
    );
    let error = count_compressed_source_observations_to_delete_at(
        &conn,
        now,
        COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
    )
    .expect_err("unknown observation columns must fail closed");
    assert!(error.to_string().contains("future_provenance"), "{error:#}");
    assert!(cleanup_compressed_source_observations_at(
        &conn,
        now,
        COMPRESSED_SOURCE_OBSERVATION_RETENTION_DAYS,
    )
    .is_err());
    assert!(observation_exists(&conn, extra_source.id));
}

pub(super) fn setup_observation_retention_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE sdk_sessions (
            id INTEGER PRIMARY KEY,
            content_session_id TEXT UNIQUE NOT NULL,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            user_prompt TEXT,
            started_at TEXT,
            started_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            prompt_counter INTEGER DEFAULT 1
        );
        CREATE TABLE observations (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            type TEXT NOT NULL,
            title TEXT,
            subtitle TEXT,
            narrative TEXT,
            facts TEXT,
            concepts TEXT,
            files_read TEXT,
            files_modified TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER,
            branch TEXT,
            commit_sha TEXT,
            host_id INTEGER,
            project_id INTEGER,
            session_row_id INTEGER,
            observation_type TEXT,
            text TEXT,
            evidence_event_ids TEXT,
            confidence REAL,
            reference_time_epoch INTEGER
        );
        CREATE TABLE compressed_observation_sources (
            id INTEGER PRIMARY KEY,
            compressed_observation_id INTEGER NOT NULL,
            source_observation_id INTEGER NOT NULL,
            source_hash TEXT NOT NULL,
            source_snapshot_json TEXT NOT NULL,
            source_created_at_epoch INTEGER NOT NULL,
            compression_session_id TEXT NOT NULL,
            created_at_epoch INTEGER NOT NULL,
            UNIQUE(compressed_observation_id, source_observation_id),
            FOREIGN KEY(compressed_observation_id) REFERENCES observations(id) ON DELETE CASCADE
        );",
    )
    .unwrap();
}

pub(super) fn observation(
    id: i64,
    status: &str,
    created_at_epoch: i64,
    title: &str,
) -> Observation {
    observation_with_content_session(id, status, created_at_epoch, title, "")
}

fn observation_with_content_session(
    id: i64,
    status: &str,
    created_at_epoch: i64,
    title: &str,
    content_session_id: &str,
) -> Observation {
    Observation {
        id,
        memory_session_id: format!("session-{id}"),
        r#type: "discovery".to_string(),
        title: Some(title.to_string()),
        subtitle: None,
        narrative: Some(format!("narrative {title}")),
        facts: None,
        concepts: None,
        files_read: None,
        files_modified: None,
        discovery_tokens: Some(1),
        created_at: format!("{created_at_epoch}"),
        created_at_epoch,
        project: Some("proj".to_string()),
        status: status.to_string(),
        last_accessed_epoch: None,
        content_session_id: (!content_session_id.is_empty())
            .then(|| content_session_id.to_string()),
        branch: None,
        commit_sha: None,
    }
}

pub(super) fn insert_observation_row(conn: &Connection, observation: &Observation) {
    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, subtitle, narrative,
          facts, concepts, files_read, files_modified, created_at,
          created_at_epoch, discovery_tokens, status, last_accessed_epoch,
          branch, commit_sha)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            observation.id,
            observation.memory_session_id,
            observation.project,
            observation.r#type,
            observation.title,
            observation.subtitle,
            observation.narrative,
            observation.facts,
            observation.concepts,
            observation.files_read,
            observation.files_modified,
            observation.created_at,
            observation.created_at_epoch,
            observation.discovery_tokens,
            observation.status,
            observation.last_accessed_epoch,
            observation.branch,
            observation.commit_sha
        ],
    )
    .unwrap();
    if let Some(content_session_id) = &observation.content_session_id {
        conn.execute(
            "INSERT INTO sdk_sessions (content_session_id, memory_session_id, project)
             VALUES (?1, ?2, ?3)",
            params![
                content_session_id,
                observation.memory_session_id,
                observation.project
            ],
        )
        .unwrap();
    }
}

pub(super) fn link_source(
    conn: &Connection,
    compressed_observation_id: i64,
    source: &Observation,
    created_at_epoch: i64,
) {
    let original_status = source.status.clone();
    conn.execute(
        "UPDATE observations SET status = 'active' WHERE id = ?1",
        params![source.id],
    )
    .unwrap();
    let mut active_source = source.clone();
    active_source.status = "active".to_string();
    crate::db::insert_compressed_observation_sources(
        conn,
        &[compressed_observation_id],
        std::slice::from_ref(&active_source),
        "compressed-session",
    )
    .unwrap();
    conn.execute(
        "UPDATE observations SET status = ?1 WHERE id = ?2",
        params![original_status, source.id],
    )
    .unwrap();
    conn.execute(
        "UPDATE compressed_observation_sources SET created_at_epoch = ?1
         WHERE compressed_observation_id = ?2 AND source_observation_id = ?3",
        params![created_at_epoch, compressed_observation_id, source.id],
    )
    .unwrap();
}

pub(super) fn rewrite_source_link_as_v1(conn: &Connection, source: &Observation) {
    conn.execute(
        "UPDATE compressed_observation_sources
         SET source_hash = ?1, source_snapshot_json = ?2
         WHERE source_observation_id = ?3",
        params![
            crate::db::observation_source_hash(source),
            crate::db::observation_source_snapshot_json(source).unwrap(),
            source.id
        ],
    )
    .unwrap();
}

pub(super) fn observation_exists(conn: &Connection, id: i64) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM observations WHERE id = ?1)",
        params![id],
        |row| row.get(0),
    )
    .unwrap()
}

fn source_link_count(conn: &Connection, source_observation_id: i64) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM compressed_observation_sources WHERE source_observation_id = ?1",
        params![source_observation_id],
        |row| row.get(0),
    )
    .unwrap()
}
