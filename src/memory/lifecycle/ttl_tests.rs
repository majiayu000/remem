use anyhow::{Context, Result};
use rusqlite::Connection;

use super::*;
use crate::memory::insert_memory;
use crate::memory::tests_helper::setup_memory_schema;
use crate::retrieval::search::search_with_branch;

#[test]
fn ttl_classifier_assigns_current_fact_windows_only() {
    assert_eq!(
        default_ttl_seconds(
            "discovery",
            Some("repo:/tmp/app:dev-server"),
            "Local dev server is currently running on the current branch at localhost:3000.",
        ),
        Some(SHORT_CURRENT_TTL_SECONDS)
    );
    assert_eq!(
        default_ttl_seconds(
            "discovery",
            Some("repo:/tmp/app:git-divergence"),
            "Current branch is ahead of origin/main by two commits.",
        ),
        Some(BRANCH_SNAPSHOT_TTL_SECONDS)
    );
    assert_eq!(
        default_ttl_seconds(
            "architecture",
            Some("repo:/tmp/app:storage-design"),
            "Use SQLite WAL for concurrent readers.",
        ),
        None
    );
    assert_eq!(
        default_ttl_seconds(
            "architecture",
            Some("repo:/tmp/app:storage-design"),
            "The architecture decision mentions localhost examples and pull request checks.",
        ),
        None
    );
    assert_eq!(
        default_ttl_seconds(
            "procedure",
            Some("repo:/tmp/app:release-procedure"),
            "When reviewing pull requests, run CI status checks before merge.",
        ),
        None
    );
    assert_eq!(
        default_ttl_seconds(
            "preference",
            Some("user:default:communication-style"),
            "User prefers concise Chinese progress updates.",
        ),
        None
    );
}

#[test]
fn apply_add_assigns_ttl_to_current_operational_fact() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle";
    let before = chrono::Utc::now().timestamp();

    let outcome = apply_add(
        &conn,
        Some("s1"),
        project,
        Some("repo:test-lifecycle:dev-server"),
        "Dev server",
        "Local dev server is currently running at localhost:3000.",
        "discovery",
        None,
        None,
        "project",
    )?;
    let memory_id = outcome.memory_id.expect("memory id");
    let (expires_at, valid_from): (i64, i64) = conn.query_row(
        "SELECT expires_at_epoch, valid_from_epoch FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;

    let min_expected = before + SHORT_CURRENT_TTL_SECONDS;
    let max_expected = chrono::Utc::now().timestamp() + SHORT_CURRENT_TTL_SECONDS;
    assert!((min_expected..=max_expected).contains(&expires_at));
    assert!(valid_from >= before);
    Ok(())
}

#[test]
fn ttl_expiry_marks_current_fact_stale_without_deleting_it() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle";
    let id = insert_memory(
        &conn,
        Some("s1"),
        project,
        Some("repo:test-lifecycle:dev-server"),
        "Dev server",
        "Local dev server is currently running at localhost:3000.",
        "discovery",
        None,
    )?;
    conn.execute(
        "UPDATE memories
         SET expires_at_epoch = ?1, valid_from_epoch = ?2
         WHERE id = ?3",
        rusqlite::params![100, 50, id],
    )?;

    let changed = expire_active_memories(&conn, 101)?;
    assert_eq!(changed, 1);
    let (status, valid_to, content): (String, Option<i64>, String) = conn.query_row(
        "SELECT status, valid_to_epoch, content FROM memories WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(status, "stale");
    assert_eq!(valid_to, Some(101));
    assert_eq!(
        content,
        "Local dev server is currently running at localhost:3000."
    );
    Ok(())
}

#[test]
fn default_search_excludes_expired_active_memory_but_debug_lookup_can_inspect_it() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle";
    let expired_id = insert_memory(
        &conn,
        Some("s1"),
        project,
        Some("repo:test-lifecycle:dev-server"),
        "Expired dev server",
        "localhost server old-state-needle is currently running at port 3000.",
        "discovery",
        None,
    )?;
    let fresh_id = insert_memory(
        &conn,
        Some("s2"),
        project,
        Some("repo:test-lifecycle:server-policy"),
        "Durable server policy",
        "server policy old-state-needle uses fixed configuration.",
        "architecture",
        None,
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE memories SET expires_at_epoch = ?1 WHERE id = ?2",
        rusqlite::params![now - 1, expired_id],
    )?;

    let current = search_with_branch(
        &conn,
        Some("old-state-needle"),
        Some(project),
        None,
        10,
        0,
        false,
        None,
    )?;
    assert_eq!(
        current.iter().map(|memory| memory.id).collect::<Vec<_>>(),
        vec![fresh_id]
    );

    let debug = crate::memory::get_memories_by_ids(&conn, &[expired_id], Some(project))?;
    assert_eq!(debug.len(), 1);
    assert_eq!(debug[0].status, "active");
    Ok(())
}

#[test]
fn durable_memory_without_ttl_survives_expiry_job() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle";
    let id = insert_memory(
        &conn,
        Some("s1"),
        project,
        Some("repo:test-lifecycle:storage-design"),
        "Storage design",
        "Use SQLite WAL for concurrent readers.",
        "architecture",
        None,
    )?;

    assert_eq!(expire_active_memories(&conn, i64::MAX)?, 0);
    let status: String =
        conn.query_row("SELECT status FROM memories WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    assert_eq!(status, "active");
    Ok(())
}

#[test]
fn state_key_update_inserts_replacement_and_stales_old_fact() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle";
    let state_key = "repo:test-lifecycle:dev-server";
    let old_id = insert_memory(
        &conn,
        Some("s1"),
        project,
        Some(state_key),
        "Dev server old",
        "Dev server is running on port 3000.",
        "discovery",
        None,
    )?;

    let outcome = apply_update(
        &conn,
        Some("s2"),
        project,
        state_key,
        "Dev server current",
        "Dev server is running on port 5173.",
        "discovery",
        None,
        None,
        "project",
        &[],
    )?;
    let new_id = outcome.memory_id.expect("replacement id");

    assert_ne!(old_id, new_id);
    assert_eq!(outcome.superseded, 1);
    let rows = conn
        .prepare(
            "SELECT id, status, topic_key, content, expires_at_epoch, valid_from_epoch
             FROM memories
             WHERE topic_key = ?1 ORDER BY id ASC",
        )?
        .query_map([state_key], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, "stale");
    assert_eq!(rows[1].1, "active");
    assert_eq!(rows[1].3, "Dev server is running on port 5173.");
    assert!(rows[1].4.is_some());
    assert!(rows[1].5.is_some());
    Ok(())
}

#[test]
fn exact_topic_update_still_replaces_legacy_memory_without_state_key() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle";
    let topic_key = "repo:test-lifecycle:legacy-dev-server";
    let old_id = insert_memory(
        &conn,
        Some("s1"),
        project,
        Some(topic_key),
        "Dev server old",
        "Dev server is running on port 3000.",
        "discovery",
        None,
    )?;
    conn.execute(
        "UPDATE memories SET state_key_id = NULL WHERE id = ?1",
        [old_id],
    )?;
    conn.execute("DELETE FROM memory_state_keys", [])?;

    let outcome = apply_update(
        &conn,
        Some("s2"),
        project,
        topic_key,
        "Dev server current",
        "Dev server is running on port 5173.",
        "discovery",
        None,
        None,
        "project",
        &[],
    )?;
    let new_id = outcome.memory_id.context("replacement id")?;

    assert_eq!(outcome.superseded, 1);
    let old_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        [old_id],
        |row| row.get(0),
    )?;
    let new_state_key_id: Option<i64> = conn.query_row(
        "SELECT state_key_id FROM memories WHERE id = ?1",
        [new_id],
        |row| row.get(0),
    )?;
    assert_eq!(old_status, "stale");
    assert!(new_state_key_id.is_some());
    Ok(())
}

#[test]
fn hash_like_topic_update_replaces_same_semantic_state_key_and_moves_current_pointer() -> Result<()>
{
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle";
    let add = apply_add(
        &conn,
        Some("s1"),
        project,
        Some("preference-aaaaaaaa"),
        "Preference",
        "Keep verification status separate from data and code changes.",
        "preference",
        None,
        None,
        "global",
    )?;
    let old_id = add.memory_id.context("old memory id")?;

    let outcome = apply_update(
        &conn,
        Some("s2"),
        project,
        "preference-bbbbbbbb",
        "Preference",
        "Report data and code changes separately from verification status.",
        "preference",
        None,
        None,
        "global",
        &[],
    )?;
    let new_id = outcome.memory_id.context("replacement id")?;

    assert_ne!(old_id, new_id);
    assert_eq!(outcome.superseded, 1);
    let (old_status, new_status): (String, String) = conn.query_row(
        "SELECT old.status, new.status
         FROM memories old
         JOIN memories new ON new.id = ?2
         WHERE old.id = ?1",
        rusqlite::params![old_id, new_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(old_status, "stale");
    assert_eq!(new_status, "active");

    let (state_key_id, state_key, current_memory_id): (i64, String, i64) = conn.query_row(
        "SELECT sk.id, sk.state_key, sk.current_memory_id
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.id = ?1",
        [new_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(state_key, "verification-status-separation");
    assert_eq!(current_memory_id, new_id);

    let old_state_key_id: i64 = conn.query_row(
        "SELECT state_key_id FROM memories WHERE id = ?1",
        [old_id],
        |row| row.get(0),
    )?;
    assert_eq!(old_state_key_id, state_key_id);
    Ok(())
}
