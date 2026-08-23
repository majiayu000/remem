use anyhow::{bail, Result};
use rusqlite::Connection;

use super::*;

fn add_request(activation_id: &str, content: &str) -> ActiveMemoryWriteRequest {
    ActiveMemoryWriteRequest {
        activation_id: activation_id.to_string(),
        route_kind: ActivationRouteKind::PackImport,
        actor_kind: ActivationActorKind::Operator,
        source_operation: "pack_import_safe_add".to_string(),
        source_trust: SourceTrustClass::Pack,
        result_source_trust: SourceTrustClass::Pack,
        source_project: "/repo".to_string(),
        route: ActiveMemoryRoute::default_for("/repo", None, "project"),
        provenance_kind: ActivationProvenanceKind::Pack,
        provenance_ref: format!("pack:test:{activation_id}"),
        payload_sha256: payload_sha256(&[content]),
        expected_memory: ExpectedActiveMemory::new("title", content, "discovery"),
        poisoning_verdict: ActivationPoisoningVerdict::UpstreamValidated,
        superseded_ids: Vec::new(),
    }
}

fn insert_pack_memory(
    conn: &Connection,
    _permit: &ActiveMemoryWritePermit,
    content: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO memories
         (project, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, target_project,
          owner_scope, owner_key, context_class, source_trust_class)
         VALUES ('/repo', 'title', ?1, 'discovery', 1, 1, 'active',
                 'project', '/repo', '/repo', 'repo', '/repo',
                 'startup_core', 'pack')",
        [content],
    )?;
    Ok(conn.last_insert_rowid())
}

#[test]
fn add_batch_records_individual_receipts_and_replays_without_writes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let requests = [
        add_request("pack:first", "first"),
        add_request("pack:second", "second"),
    ];
    let contents = ["first", "second"];

    let first = execute_add_batch(&conn, &requests, |index, permit| {
        insert_pack_memory(&conn, permit, contents[index])
    })?;
    assert_eq!(first.len(), 2);
    assert!(first.iter().all(|result| !result.replayed));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        2
    );

    let replay = execute_add_batch(&conn, &requests, |_, _| bail!("writer must not replay"))?;
    assert!(replay.iter().all(|result| result.replayed));
    assert_eq!(
        replay
            .iter()
            .map(|result| result.memory_id)
            .collect::<Vec<_>>(),
        first
            .iter()
            .map(|result| result.memory_id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn add_batch_rolls_back_undeclared_active_writes() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let requests = [add_request("pack:declared", "declared")];

    let error = execute_add_batch(&conn, &requests, |_, permit| {
        let declared = insert_pack_memory(&conn, permit, "declared")?;
        insert_pack_memory(&conn, permit, "undeclared")?;
        Ok(declared)
    })
    .expect_err("undeclared batch write must fail");
    assert!(error.to_string().contains("result/addition drift"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        0
    );
    Ok(())
}

#[test]
fn add_batch_rolls_back_in_place_update_returned_as_addition() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let seed_request = add_request("pack:seed", "seed");
    let seed = execute_add_batch(&conn, &[seed_request], |_, permit| {
        insert_pack_memory(&conn, permit, "seed")
    })?
    .remove(0);
    let update_request = add_request("pack:in-place", "changed");

    let error = execute_add_batch(&conn, &[update_request], |_, _| {
        conn.execute(
            "UPDATE memories SET content = 'changed' WHERE id = ?1",
            [seed.memory_id],
        )?;
        Ok(seed.memory_id)
    })
    .expect_err("an existing row cannot masquerade as an add result");
    assert!(error.to_string().contains("result/addition drift"));
    assert_eq!(
        conn.query_row(
            "SELECT content FROM memories WHERE id = ?1",
            [seed.memory_id],
            |row| row.get::<_, String>(0),
        )?,
        "seed"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        1
    );
    Ok(())
}

#[test]
fn add_batch_rolls_back_when_later_writer_changes_earlier_result() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let requests = [
        add_request("pack:first-stable", "first"),
        add_request("pack:second-mutator", "second"),
    ];
    let mut first_id = None;

    let error = execute_add_batch(&conn, &requests, |index, permit| {
        if index == 0 {
            let memory_id = insert_pack_memory(&conn, permit, "first")?;
            first_id = Some(memory_id);
            return Ok(memory_id);
        }
        conn.execute(
            "UPDATE memories SET content = 'changed later' WHERE id = ?1",
            [first_id.context("first batch result")?],
        )?;
        insert_pack_memory(&conn, permit, "second")
    })
    .expect_err("later writer must not change an earlier batch result");
    assert!(error
        .to_string()
        .contains("result payload does not match reviewed request"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM memory_activation_requests",
            [],
            |row| { row.get::<_, i64>(0) }
        )?,
        0
    );
    Ok(())
}
