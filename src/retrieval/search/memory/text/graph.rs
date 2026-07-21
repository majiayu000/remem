use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use crate::perf::time_result;
use crate::retrieval::search::common::{rank_normalized_score, WeightedRankedHit};

use super::super::{suppression_filter, SearchWeights};
use super::NamedChannel;

#[allow(clippy::too_many_arguments)]
pub(super) fn append_graph_channel(
    conn: &Connection,
    channels: &mut Vec<NamedChannel>,
    timings: &mut Vec<crate::perf::PhaseTiming>,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    include_stale: bool,
    include_suppressed: bool,
    fetch_limit: i64,
    weights: SearchWeights,
) -> Result<()> {
    if weights.graph <= 0.0 {
        channels.push(NamedChannel::disabled(
            "graph_traversal",
            weights.graph,
            "graph channel weight is zero",
        ));
        return Ok(());
    }

    let seed_ids = seed_ids(channels, 32);
    let outcome = time_result(timings, "graph_traversal", || {
        crate::retrieval::graph::traverse_trusted_graph(
            conn,
            crate::retrieval::graph::GraphTraversalRequest {
                seed_memory_ids: &seed_ids,
                project,
                memory_type,
                branch,
                include_inactive: include_stale,
                reference_time_epoch: chrono::Utc::now().timestamp(),
                limits: crate::retrieval::graph::GraphTraversalLimits::for_search(fetch_limit),
            },
        )
    })?;
    if let Some(reason) = outcome.status.disabled_reason() {
        channels.push(
            NamedChannel::disabled("graph_traversal", weights.graph, reason)
                .with_candidates_scanned(outcome.diagnostics.edges_scanned),
        );
        return Ok(());
    }

    let hits = outcome
        .hits
        .into_iter()
        .enumerate()
        .map(|(rank, hit)| WeightedRankedHit {
            id: hit.memory_id,
            normalized_score: rank_normalized_score(rank),
        })
        .collect::<Vec<_>>();
    let hits = suppression_filter::weighted_hits(conn, hits, include_suppressed)?;
    channels.push(graph_channel_after_suppression(
        hits,
        weights.graph,
        outcome.diagnostics.edges_scanned,
    ));
    Ok(())
}

fn graph_channel_after_suppression(
    hits: Vec<WeightedRankedHit>,
    weight: f64,
    candidates_scanned: usize,
) -> NamedChannel {
    if hits.is_empty() {
        return NamedChannel::disabled(
            "graph_traversal",
            weight,
            "all eligible graph candidates were suppressed",
        )
        .with_candidates_scanned(candidates_scanned);
    }
    NamedChannel::enabled_with_hits("graph_traversal", weight, hits)
        .with_candidates_scanned(candidates_scanned)
}

fn seed_ids(channels: &[NamedChannel], max_seeds: usize) -> Vec<i64> {
    let mut seen = HashSet::new();
    channels
        .iter()
        .filter(|channel| channel.name == "fts" || channel.name == "vector")
        .flat_map(|channel| channel.hits.iter().map(|hit| hit.id))
        .filter(|id| seen.insert(*id))
        .take(max_seeds)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    use crate::memory::graph_contract::{
        insert_graph_edge, GraphEdgeInput, GraphEdgeProvenance, GraphEdgeType, GraphNodeRef,
    };

    #[test]
    fn post_suppression_empty_graph_channel_has_stable_disabled_reason() {
        let channel =
            graph_channel_after_suppression(Vec::new(), SearchWeights::default().graph, 2);

        assert!(!channel.is_enabled());
        assert!(channel.hits.is_empty());
        assert_eq!(channel.candidates_scanned, Some(2));
        assert_eq!(
            channel.disabled_reason.as_deref(),
            Some("all eligible graph candidates were suppressed")
        );
    }

    #[test]
    fn seed_ids_come_only_from_fts_and_vector_are_deduped_and_capped() {
        let channels = vec![
            NamedChannel::enabled("fts", 1.0, vec![10, 11, 10]),
            NamedChannel::enabled("vector", 1.0, vec![11, 12]),
            NamedChannel::enabled("entity", 1.0, vec![99]),
            NamedChannel::disabled("temporal", 1.0, "no range"),
        ];
        assert_eq!(seed_ids(&channels, 32), vec![10, 11, 12]);
        assert_eq!(seed_ids(&channels, 2), vec![10, 11]);
    }

    #[test]
    fn graph_channel_after_suppression_preserves_unsuppressed_hits() {
        let hits = vec![
            WeightedRankedHit {
                id: 7,
                normalized_score: 1.0,
            },
            WeightedRankedHit {
                id: 9,
                normalized_score: 0.5,
            },
        ];
        let channel = graph_channel_after_suppression(hits, 0.75, 4);
        assert!(channel.is_enabled());
        assert_eq!(
            channel.hits.iter().map(|hit| hit.id).collect::<Vec<_>>(),
            vec![7, 9]
        );
        assert_eq!(channel.candidates_scanned, Some(4));
        assert!(channel.disabled_reason.is_none());
    }

    #[test]
    fn append_graph_channel_records_missing_table_disabled_reason_and_timing() -> Result<()> {
        let bare = Connection::open_in_memory()?;
        let mut channels = vec![NamedChannel::enabled("fts", 1.0, vec![1])];
        let mut timings: Vec<crate::perf::PhaseTiming> = Vec::new();
        append_graph_channel(
            &bare,
            &mut channels,
            &mut timings,
            Some("/repo"),
            Some("decision"),
            Some("main"),
            false,
            false,
            10,
            SearchWeights::default(),
        )?;
        let graph = channels
            .iter()
            .find(|channel| channel.name == "graph_traversal")
            .expect("graph channel appended");
        assert!(!graph.is_enabled());
        assert_eq!(
            graph.disabled_reason.as_deref(),
            Some("graph_edges table is unavailable")
        );
        assert!(timings
            .iter()
            .any(|timing| timing.phase == "graph_traversal"));
        Ok(())
    }

    #[test]
    fn append_graph_channel_enables_real_graph_hits_and_suppression_rows_apply() -> Result<()> {
        let (conn, seed, target) = channel_fixture()?;
        let mut channels = vec![NamedChannel::enabled("fts", 1.0, vec![seed])];
        let mut timings: Vec<crate::perf::PhaseTiming> = Vec::new();
        append_graph_channel(
            &conn,
            &mut channels,
            &mut timings,
            Some("/repo"),
            Some("decision"),
            Some("main"),
            false,
            false,
            10,
            SearchWeights::default(),
        )?;
        let graph = channels
            .iter()
            .find(|channel| channel.name == "graph_traversal")
            .expect("graph channel appended");
        assert!(
            graph.is_enabled(),
            "real two-hop hit must enable the graph channel"
        );
        assert_eq!(
            graph.hits.iter().map(|hit| hit.id).collect::<Vec<_>>(),
            vec![target]
        );
        assert!(timings
            .iter()
            .any(|timing| timing.phase == "graph_traversal"));

        // A suppression row on the target must drop it and disable the channel.
        conn.execute(
            "INSERT INTO memory_suppressions (owner_scope, owner_key, target_kind, target_id,
                target_value, reason, actor, status, created_at_epoch, updated_at_epoch)
             VALUES (NULL, NULL, 'memory', ?1, NULL, 'gh900 test', 'gh900', 'active',
                1700000000, 1700000000)",
            params![target],
        )?;
        let mut channels = vec![NamedChannel::enabled("fts", 1.0, vec![seed])];
        let mut timings: Vec<crate::perf::PhaseTiming> = Vec::new();
        append_graph_channel(
            &conn,
            &mut channels,
            &mut timings,
            Some("/repo"),
            Some("decision"),
            Some("main"),
            false,
            false,
            10,
            SearchWeights::default(),
        )?;
        let graph = channels
            .iter()
            .find(|channel| channel.name == "graph_traversal")
            .expect("graph channel appended");
        assert!(!graph.is_enabled());
        assert_eq!(
            graph.disabled_reason.as_deref(),
            Some("all eligible graph candidates were suppressed")
        );
        Ok(())
    }

    fn channel_fixture() -> Result<(Connection, i64, i64)> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let now = 1_700_000_000_i64;
        let host_id: i64 =
            conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
                row.get(0)
            })?;
        conn.execute(
            "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch,
                updated_at_epoch) VALUES ('/tmp/gh900', 'origin', 'main', ?1, ?1)",
            [now],
        )?;
        let workspace_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch,
                updated_at_epoch) VALUES (?1, '/tmp/gh900', 'gh900', ?2, ?2)",
            params![workspace_id, now],
        )?;
        let project_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sessions(host_id, workspace_id, project_id, session_id,
                started_at_epoch, last_seen_at_epoch, status)
             VALUES (?1, ?2, ?3, 'gh900-session', ?4, ?4, 'active')",
            params![host_id, workspace_id, project_id, now],
        )?;
        let session_row_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO captured_events(host_id, workspace_id, project_id, session_row_id,
                session_id, event_id, event_type, content_hash, retention_class,
                created_at_epoch, inserted_at_epoch)
             VALUES (?1, ?2, ?3, ?4, 'gh900-session', 'gh900-event', 'message',
                'gh900-hash', 'default', ?5, ?5)",
            params![host_id, workspace_id, project_id, session_row_id, now],
        )?;
        let event_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
                evidence_event_ids, confidence, risk_class, review_status, created_at_epoch,
                updated_at_epoch)
             VALUES (?1, 'project', 'decision', 'gh900', 'channel fixture',
                ?2, 0.9, 'low', 'accepted', ?3, ?3)",
            params![project_id, format!("[{event_id}]"), now],
        )?;
        let candidate_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
                owner_scope, owner_key, memory_type, state_key, source_candidate_id,
                superseded_ids, conflicting_ids, confidence, reason, created_at_epoch)
             VALUES ('add', 'gh900-test', 'test', 'memory_candidate', 'project', '/repo',
                'decision', 'gh900', ?1, '[]', '[]', 0.9, 'channel fixture', ?2)",
            params![candidate_id, now],
        )?;
        let operation_id = conn.last_insert_rowid();

        let seed = crate::memory::insert_memory_full(
            &conn,
            Some("gh900-session"),
            "/repo",
            Some("seed"),
            "seed",
            "content for seed",
            "decision",
            None,
            Some("main"),
            "project",
            Some(now),
        )?;
        let target = crate::memory::insert_memory_full(
            &conn,
            Some("gh900-session"),
            "/repo",
            Some("target"),
            "target",
            "content for target",
            "decision",
            None,
            Some("main"),
            "project",
            Some(now),
        )?;

        conn.execute(
            "INSERT INTO entities(canonical_name, entity_type, mention_count, created_at_epoch)
             VALUES ('GH900 bridge', 'concept', 1, 1700000000)",
            [],
        )?;
        let entity = conn.last_insert_rowid();
        let provenance = GraphEdgeProvenance {
            source_event_ids: std::slice::from_ref(&event_id),
            source_candidate_id: Some(candidate_id),
            source_operation_id: Some(operation_id),
            confidence: Some(0.9),
            reason: Some("gh900 channel test"),
        };
        for memory_id in [seed, target] {
            insert_graph_edge(
                &conn,
                &GraphEdgeInput {
                    edge_type: GraphEdgeType::Mentions,
                    from_node: GraphNodeRef::memory(memory_id)?,
                    to_node: GraphNodeRef::entity(entity)?,
                    provenance,
                    valid_from_epoch: None,
                    valid_to_epoch: None,
                },
            )?;
        }
        Ok((conn, seed, target))
    }
}
