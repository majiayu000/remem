use anyhow::Result;
use rusqlite::{params, Connection};

use super::*;
use crate::memory::graph_contract::{
    insert_graph_edge, GraphEdgeInput, GraphEdgeProvenance, GraphEdgeType, GraphNodeRef,
};

struct Fixture {
    conn: Connection,
    event_id: i64,
    candidate_id: i64,
    operation_id: i64,
}

fn fixture() -> Result<Fixture> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = 1_700_000_000_i64;
    let host_id: i64 =
        conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
            row.get(0)
        })?;
    conn.execute(
        "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch)
         VALUES ('/tmp/gh853', 'origin', 'main', ?1, ?1)",
        [now],
    )?;
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
         VALUES (?1, '/tmp/gh853', 'gh853', ?2, ?2)",
        params![workspace_id, now],
    )?;
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, started_at_epoch,
                              last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, 'gh853-session', ?4, ?4, 'active')",
        params![host_id, workspace_id, project_id, now],
    )?;
    let session_row_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO captured_events(host_id, workspace_id, project_id, session_row_id,
                                     session_id, event_id, event_type, content_hash,
                                     retention_class, created_at_epoch, inserted_at_epoch)
         VALUES (?1, ?2, ?3, ?4, 'gh853-session', 'gh853-event', 'message',
                 'gh853-hash', 'default', ?5, ?5)",
        params![host_id, workspace_id, project_id, session_row_id, now],
    )?;
    let event_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
                                       evidence_event_ids, confidence, risk_class,
                                       review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'gh853', 'graph fixture',
                 ?2, 0.9, 'low', 'accepted', ?3, ?3)",
        params![project_id, format!("[{event_id}]"), now],
    )?;
    let candidate_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
                                         owner_scope, owner_key, memory_type, state_key,
                                         source_candidate_id, superseded_ids, conflicting_ids,
                                         confidence, reason, created_at_epoch)
         VALUES ('add', 'gh853-test', 'test', 'memory_candidate', 'project', '/repo',
                 'decision', 'gh853', ?1, '[]', '[]', 0.9, 'graph fixture', ?2)",
        params![candidate_id, now],
    )?;
    let operation_id = conn.last_insert_rowid();
    Ok(Fixture {
        conn,
        event_id,
        candidate_id,
        operation_id,
    })
}

fn add_memory(fixture: &Fixture, project: &str, topic: &str, status: &str) -> Result<i64> {
    let id = crate::memory::insert_memory_full(
        &fixture.conn,
        Some("gh853-session"),
        project,
        Some(topic),
        topic,
        &format!("content for {topic}"),
        "decision",
        None,
        Some("main"),
        "project",
        Some(1_700_000_000),
    )?;
    if status != "active" {
        fixture.conn.execute(
            "UPDATE memories SET status = ?1 WHERE id = ?2",
            params![status, id],
        )?;
    }
    Ok(id)
}

fn provenance(fixture: &Fixture) -> GraphEdgeProvenance<'_> {
    GraphEdgeProvenance {
        source_event_ids: std::slice::from_ref(&fixture.event_id),
        source_candidate_id: Some(fixture.candidate_id),
        source_operation_id: Some(fixture.operation_id),
        confidence: Some(0.9),
        reason: Some("gh853 traversal test"),
    }
}

fn request(seed_ids: &[i64]) -> GraphTraversalRequest<'_> {
    GraphTraversalRequest {
        seed_memory_ids: seed_ids,
        project: Some("/repo"),
        memory_type: Some("decision"),
        branch: Some("main"),
        include_inactive: false,
        reference_time_epoch: 1_700_000_100,
        limits: GraphTraversalLimits::default(),
    }
}

#[test]
fn reports_missing_empty_and_no_seed_states() -> Result<()> {
    let bare = Connection::open_in_memory()?;
    assert_eq!(
        traverse_trusted_graph(&bare, request(&[1]))?.status,
        GraphTraversalStatus::MissingTable
    );
    let fixture = fixture()?;
    assert_eq!(
        traverse_trusted_graph(&fixture.conn, request(&[]))?.status,
        GraphTraversalStatus::NoSeed
    );
    assert_eq!(
        traverse_trusted_graph(&fixture.conn, request(&[1]))?.status,
        GraphTraversalStatus::EmptyGraph
    );
    Ok(())
}

#[test]
fn trusted_mentions_two_hop_filters_scope_and_hints() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    let target = add_memory(&fixture, "/repo", "target", "active")?;
    let other_project = add_memory(&fixture, "/other", "private", "active")?;
    fixture.conn.execute(
        "INSERT INTO entities(canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES ('GH853 bridge', 'concept', 1, 1700000000)",
        [],
    )?;
    let entity = fixture.conn.last_insert_rowid();
    for memory_id in [seed, target, other_project] {
        insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::Mentions,
                from_node: GraphNodeRef::memory(memory_id)?,
                to_node: GraphNodeRef::entity(entity)?,
                provenance: provenance(&fixture),
                valid_from_epoch: Some(1_699_999_999),
                valid_to_epoch: None,
            },
        )?;
    }
    insert_graph_edge(
        &fixture.conn,
        &GraphEdgeInput {
            edge_type: GraphEdgeType::SimilarTo,
            from_node: GraphNodeRef::memory(seed)?,
            to_node: GraphNodeRef::memory(other_project)?,
            provenance: GraphEdgeProvenance::default(),
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;

    let outcome = traverse_trusted_graph(&fixture.conn, request(&[seed]))?;
    assert_eq!(outcome.status, GraphTraversalStatus::Ready);
    assert_eq!(
        outcome
            .hits
            .iter()
            .map(|hit| hit.memory_id)
            .collect::<Vec<_>>(),
        vec![target]
    );
    assert_eq!(outcome.hits[0].hop_count, 2);
    assert_eq!(outcome.hits[0].path_kind, GraphPathKind::Mentions);
    assert_eq!(outcome.diagnostics.diagnostic_hint_edges, 1);
    assert!(outcome.diagnostics.targets_filtered >= 1);
    Ok(())
}

#[test]
fn supersedes_only_walks_from_old_seed_to_current_memory() -> Result<()> {
    let fixture = fixture()?;
    let current = add_memory(&fixture, "/repo", "current", "active")?;
    let old = add_memory(&fixture, "/repo", "old", "stale")?;
    insert_graph_edge(
        &fixture.conn,
        &GraphEdgeInput {
            edge_type: GraphEdgeType::Supersedes,
            from_node: GraphNodeRef::memory(current)?,
            to_node: GraphNodeRef::memory(old)?,
            provenance: provenance(&fixture),
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;

    let from_old = traverse_trusted_graph(&fixture.conn, request(&[old]))?;
    assert_eq!(from_old.hits[0].memory_id, current);
    assert_eq!(from_old.hits[0].hop_count, 1);
    assert_eq!(from_old.hits[0].path_kind, GraphPathKind::Supersedes);
    let from_current = traverse_trusted_graph(&fixture.conn, request(&[current]))?;
    assert_eq!(from_current.status, GraphTraversalStatus::NoExpansion);
    Ok(())
}

#[test]
fn degree_cap_fails_closed_without_partial_hits() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    for index in 0..2 {
        let target = add_memory(&fixture, "/repo", &format!("target-{index}"), "active")?;
        insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::Supersedes,
                from_node: GraphNodeRef::memory(target)?,
                to_node: GraphNodeRef::memory(seed)?,
                provenance: provenance(&fixture),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
    }
    let seeds = [seed];
    let mut bounded = request(&seeds);
    bounded.limits.max_degree_per_node = 1;
    let error = traverse_trusted_graph(&fixture.conn, bounded).unwrap_err();
    assert!(error.to_string().contains("degree cap exceeded"));
    Ok(())
}

#[test]
fn expired_edges_are_not_traversed() -> Result<()> {
    let fixture = fixture()?;
    let current = add_memory(&fixture, "/repo", "current", "active")?;
    let old = add_memory(&fixture, "/repo", "old", "stale")?;
    insert_graph_edge(
        &fixture.conn,
        &GraphEdgeInput {
            edge_type: GraphEdgeType::Supersedes,
            from_node: GraphNodeRef::memory(current)?,
            to_node: GraphNodeRef::memory(old)?,
            provenance: provenance(&fixture),
            valid_from_epoch: None,
            valid_to_epoch: Some(1_700_000_050),
        },
    )?;

    let outcome = traverse_trusted_graph(&fixture.conn, request(&[old]))?;
    assert_eq!(outcome.status, GraphTraversalStatus::NoExpansion);
    assert!(outcome.hits.is_empty());
    Ok(())
}

#[test]
fn duplicate_paths_keep_the_deterministic_highest_confidence_hit() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    let target = add_memory(&fixture, "/repo", "target", "active")?;
    for (index, confidence) in [0.55, 0.85].into_iter().enumerate() {
        fixture.conn.execute(
            "INSERT INTO entities(canonical_name, entity_type, mention_count, created_at_epoch)
             VALUES (?1, 'concept', 1, 1700000000)",
            [format!("GH853 bridge {index}")],
        )?;
        let entity = fixture.conn.last_insert_rowid();
        for memory_id in [seed, target] {
            let mut edge_provenance = provenance(&fixture);
            edge_provenance.confidence = Some(confidence);
            insert_graph_edge(
                &fixture.conn,
                &GraphEdgeInput {
                    edge_type: GraphEdgeType::Mentions,
                    from_node: GraphNodeRef::memory(memory_id)?,
                    to_node: GraphNodeRef::entity(entity)?,
                    provenance: edge_provenance,
                    valid_from_epoch: None,
                    valid_to_epoch: None,
                },
            )?;
        }
    }

    let outcome = traverse_trusted_graph(&fixture.conn, request(&[seed]))?;
    assert_eq!(outcome.status, GraphTraversalStatus::Ready);
    assert_eq!(outcome.hits.len(), 1);
    assert_eq!(outcome.hits[0].memory_id, target);
    assert_eq!(outcome.hits[0].path_kind, GraphPathKind::Mentions);
    assert_eq!(outcome.hits[0].min_confidence, 0.85);
    Ok(())
}
