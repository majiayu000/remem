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

fn add_memory_with(
    fixture: &Fixture,
    project: &str,
    topic: &str,
    memory_type: &str,
    branch: &str,
    status: &str,
) -> Result<i64> {
    let id = crate::memory::insert_memory_full(
        &fixture.conn,
        Some("gh853-session"),
        project,
        Some(topic),
        topic,
        &format!("content for {topic}"),
        memory_type,
        None,
        Some(branch),
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

fn add_entity(fixture: &Fixture, name: &str) -> Result<i64> {
    fixture.conn.execute(
        "INSERT INTO entities(canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES (?1, 'concept', 1, 1700000000)",
        params![name],
    )?;
    Ok(fixture.conn.last_insert_rowid())
}

fn add_file_node(fixture: &Fixture, project: &str, path: &str) -> Result<i64> {
    fixture.conn.execute(
        "INSERT INTO graph_file_nodes(project_id, source_project, path, created_at_epoch,
                                      updated_at_epoch)
         VALUES (NULL, ?1, ?2, 1700000000, 1700000000)",
        params![project, path],
    )?;
    Ok(fixture.conn.last_insert_rowid())
}

fn link_mentions(fixture: &Fixture, memory_id: i64, entity: i64) -> Result<()> {
    insert_graph_edge(
        &fixture.conn,
        &GraphEdgeInput {
            edge_type: GraphEdgeType::Mentions,
            from_node: GraphNodeRef::memory(memory_id)?,
            to_node: GraphNodeRef::entity(entity)?,
            provenance: provenance(fixture),
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;
    Ok(())
}

fn link_touches_file(fixture: &Fixture, memory_id: i64, file: i64) -> Result<()> {
    insert_graph_edge(
        &fixture.conn,
        &GraphEdgeInput {
            edge_type: GraphEdgeType::TouchesFile,
            from_node: GraphNodeRef::memory(memory_id)?,
            to_node: GraphNodeRef::file(file)?,
            provenance: provenance(fixture),
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;
    Ok(())
}

#[test]
fn trusted_touches_file_two_hop_reaches_co_touched_memory() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    let target = add_memory(&fixture, "/repo", "target", "active")?;
    let file = add_file_node(&fixture, "/repo", "src/bridge.rs")?;
    link_touches_file(&fixture, seed, file)?;
    link_touches_file(&fixture, target, file)?;

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
    assert_eq!(outcome.hits[0].path_kind, GraphPathKind::TouchesFile);
    Ok(())
}

#[test]
fn eligibility_filters_parameterize_memory_type_branch_and_status() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    let eligible = add_memory(&fixture, "/repo", "eligible", "active")?;
    let wrong_type = add_memory_with(&fixture, "/repo", "wrong-type", "note", "main", "active")?;
    let wrong_branch = add_memory_with(
        &fixture,
        "/repo",
        "wrong-branch",
        "decision",
        "feature",
        "active",
    )?;
    let inactive = add_memory_with(&fixture, "/repo", "inactive", "decision", "main", "stale")?;
    for (index, target) in [eligible, wrong_type, wrong_branch, inactive]
        .into_iter()
        .enumerate()
    {
        let entity = add_entity(&fixture, &format!("bridge-{index}"))?;
        link_mentions(&fixture, seed, entity)?;
        link_mentions(&fixture, target, entity)?;
    }

    // Default request: memory_type=decision, branch=main, active only.
    let outcome = traverse_trusted_graph(&fixture.conn, request(&[seed]))?;
    assert_eq!(
        outcome
            .hits
            .iter()
            .map(|hit| hit.memory_id)
            .collect::<Vec<_>>(),
        vec![eligible]
    );
    assert_eq!(outcome.diagnostics.targets_filtered, 3);

    // Relax memory_type: the note-typed memory becomes eligible.
    let seeds = [seed];
    let mut any_type = request(&seeds);
    any_type.memory_type = None;
    let outcome = traverse_trusted_graph(&fixture.conn, any_type)?;
    let ids = outcome
        .hits
        .iter()
        .map(|hit| hit.memory_id)
        .collect::<Vec<_>>();
    assert!(ids.contains(&eligible));
    assert!(ids.contains(&wrong_type));
    assert!(!ids.contains(&wrong_branch));
    assert!(!ids.contains(&inactive));

    // Relax memory_type + branch: the feature-branch memory becomes eligible.
    let mut any_branch = request(&seeds);
    any_branch.memory_type = None;
    any_branch.branch = None;
    let outcome = traverse_trusted_graph(&fixture.conn, any_branch)?;
    let ids = outcome
        .hits
        .iter()
        .map(|hit| hit.memory_id)
        .collect::<Vec<_>>();
    assert!(ids.contains(&wrong_branch));
    assert!(!ids.contains(&inactive));

    // include_inactive admits the stale memory as well.
    let mut include_inactive = request(&seeds);
    include_inactive.memory_type = None;
    include_inactive.branch = None;
    include_inactive.include_inactive = true;
    let outcome = traverse_trusted_graph(&fixture.conn, include_inactive)?;
    let ids = outcome
        .hits
        .iter()
        .map(|hit| hit.memory_id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 4);
    assert!(ids.contains(&inactive));
    Ok(())
}

#[test]
fn seed_cap_fails_closed_before_any_expansion() -> Result<()> {
    let fixture = fixture()?;
    let seeds = [1, 2, 3];
    let mut bounded = request(&seeds);
    bounded.limits.max_seeds = 2;
    let error = traverse_trusted_graph(&fixture.conn, bounded).unwrap_err();
    assert!(error.to_string().contains("seed cap exceeded"));
    Ok(())
}

#[test]
fn candidate_cap_fails_closed_without_partial_hits() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    for index in 0..2 {
        let target = add_memory(&fixture, "/repo", &format!("target-{index}"), "active")?;
        let entity = add_entity(&fixture, &format!("bridge-{index}"))?;
        link_mentions(&fixture, seed, entity)?;
        link_mentions(&fixture, target, entity)?;
    }
    let seeds = [seed];
    let mut bounded = request(&seeds);
    bounded.limits.max_candidates = 1;
    let error = traverse_trusted_graph(&fixture.conn, bounded).unwrap_err();
    assert!(error.to_string().contains("candidate cap exceeded"));
    Ok(())
}

#[test]
fn edge_scan_cap_fails_closed() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    for index in 0..2 {
        let target = add_memory(&fixture, "/repo", &format!("target-{index}"), "active")?;
        let entity = add_entity(&fixture, &format!("bridge-{index}"))?;
        link_mentions(&fixture, seed, entity)?;
        link_mentions(&fixture, target, entity)?;
    }
    let seeds = [seed];
    let mut bounded = request(&seeds);
    bounded.limits.max_edges_scanned = 1;
    let error = traverse_trusted_graph(&fixture.conn, bounded).unwrap_err();
    assert!(error.to_string().contains("edge scan cap exceeded"));
    Ok(())
}

#[test]
fn diamond_paths_yield_stable_deterministic_ordering() -> Result<()> {
    let fixture = fixture()?;
    let seed_a = add_memory(&fixture, "/repo", "seed-a", "active")?;
    let seed_b = add_memory(&fixture, "/repo", "seed-b", "active")?;
    let target_x = add_memory(&fixture, "/repo", "target-x", "active")?;
    let target_y = add_memory(&fixture, "/repo", "target-y", "active")?;
    // Two overlapping bridges form a diamond between the seeds and the targets.
    let bridge_one = add_entity(&fixture, "bridge-one")?;
    let bridge_two = add_entity(&fixture, "bridge-two")?;
    for memory_id in [seed_a, seed_b, target_x] {
        link_mentions(&fixture, memory_id, bridge_one)?;
    }
    for memory_id in [seed_a, seed_b, target_y] {
        link_mentions(&fixture, memory_id, bridge_two)?;
    }

    let outcome = traverse_trusted_graph(&fixture.conn, request(&[seed_a, seed_b]))?;
    assert_eq!(outcome.status, GraphTraversalStatus::Ready);
    let ids = outcome
        .hits
        .iter()
        .map(|hit| hit.memory_id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&target_x));
    assert!(ids.contains(&target_y));
    // Re-running must reproduce the identical ordering (compare_hits is total).
    let again = traverse_trusted_graph(&fixture.conn, request(&[seed_a, seed_b]))?;
    assert_eq!(
        ids,
        again
            .hits
            .iter()
            .map(|hit| hit.memory_id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn cyclic_mention_graph_terminates_and_excludes_seeds() -> Result<()> {
    let fixture = fixture()?;
    let first = add_memory(&fixture, "/repo", "first", "active")?;
    let second = add_memory(&fixture, "/repo", "second", "active")?;
    let bridge = add_entity(&fixture, "cycle-bridge")?;
    link_mentions(&fixture, first, bridge)?;
    link_mentions(&fixture, second, bridge)?;

    let outcome = traverse_trusted_graph(&fixture.conn, request(&[first]))?;
    assert_eq!(outcome.status, GraphTraversalStatus::Ready);
    assert_eq!(
        outcome
            .hits
            .iter()
            .map(|hit| hit.memory_id)
            .collect::<Vec<_>>(),
        vec![second]
    );
    Ok(())
}

#[test]
fn traversal_is_read_only_and_counts_ignored_trusted_edge_types() -> Result<()> {
    let fixture = fixture()?;
    let seed = add_memory(&fixture, "/repo", "seed", "active")?;
    let target = add_memory(&fixture, "/repo", "target", "active")?;
    let bridge = add_entity(&fixture, "bridge")?;
    link_mentions(&fixture, seed, bridge)?;
    link_mentions(&fixture, target, bridge)?;
    // A trusted edge type the traversal does not expand (duplicates) must be
    // decoded and counted as ignored, not treated as an error.
    insert_graph_edge(
        &fixture.conn,
        &GraphEdgeInput {
            edge_type: GraphEdgeType::Duplicates,
            from_node: GraphNodeRef::memory(seed)?,
            to_node: GraphNodeRef::memory(target)?,
            provenance: provenance(&fixture),
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )?;

    let edges_before: i64 =
        fixture
            .conn
            .query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    let memories_before: i64 =
        fixture
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;

    let outcome = traverse_trusted_graph(&fixture.conn, request(&[seed]))?;
    assert_eq!(
        outcome
            .hits
            .iter()
            .map(|hit| hit.memory_id)
            .collect::<Vec<_>>(),
        vec![target]
    );
    assert_eq!(outcome.diagnostics.ignored_trusted_edges, 1);

    let edges_after: i64 =
        fixture
            .conn
            .query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    let memories_after: i64 =
        fixture
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(
        edges_before, edges_after,
        "traversal must not write graph_edges"
    );
    assert_eq!(
        memories_before, memories_after,
        "traversal must not write memories"
    );
    Ok(())
}

// The traversal's "invalid bridge kind" and "unknown trust" branches are
// defense-in-depth: the v034 migration enforces edge structure with a CHECK
// constraint (edge_type vs node kinds) and `edge_trust IN ('trusted',
// 'diagnostic_hint')`, plus node-existence triggers, so those states cannot be
// produced through the graph contract. The reachable error behavior is the
// fail-closed limit validation below (and the cap tests above).
#[test]
fn non_positive_limits_fail_validation() -> Result<()> {
    let fixture = fixture()?;
    let seeds = [1];
    let mut bounded = request(&seeds);
    bounded.limits.max_seeds = 0;
    let error = traverse_trusted_graph(&fixture.conn, bounded).unwrap_err();
    assert!(error.to_string().contains("max_seeds must be positive"));
    Ok(())
}

#[test]
fn disabled_reasons_are_stable_for_each_status() {
    assert_eq!(GraphTraversalStatus::Ready.disabled_reason(), None);
    assert_eq!(
        GraphTraversalStatus::MissingTable.disabled_reason(),
        Some("graph_edges table is unavailable")
    );
    assert_eq!(
        GraphTraversalStatus::EmptyGraph.disabled_reason(),
        Some("graph_edges table is empty")
    );
    assert_eq!(
        GraphTraversalStatus::NoSeed.disabled_reason(),
        Some("no eligible FTS/vector graph seeds")
    );
    assert_eq!(
        GraphTraversalStatus::NoExpansion.disabled_reason(),
        Some("no eligible trusted graph expansion")
    );
}
