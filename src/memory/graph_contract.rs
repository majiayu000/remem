use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphNodeKind {
    Memory,
    Entity,
    Fact,
    Episode,
    State,
    Topic,
    File,
}

impl GraphNodeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Entity => "entity",
            Self::Fact => "fact",
            Self::Episode => "episode",
            Self::State => "state",
            Self::Topic => "topic",
            Self::File => "file",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphNodeRef {
    pub kind: GraphNodeKind,
    pub id: i64,
}

impl GraphNodeRef {
    pub fn new(kind: GraphNodeKind, id: i64) -> Result<Self> {
        if id <= 0 {
            bail!("graph node id must be positive");
        }
        Ok(Self { kind, id })
    }

    pub fn memory(id: i64) -> Result<Self> {
        Self::new(GraphNodeKind::Memory, id)
    }

    pub fn entity(id: i64) -> Result<Self> {
        Self::new(GraphNodeKind::Entity, id)
    }

    pub fn fact(id: i64) -> Result<Self> {
        Self::new(GraphNodeKind::Fact, id)
    }

    pub fn episode(id: i64) -> Result<Self> {
        Self::new(GraphNodeKind::Episode, id)
    }

    pub fn state(id: i64) -> Result<Self> {
        Self::new(GraphNodeKind::State, id)
    }

    pub fn topic(id: i64) -> Result<Self> {
        Self::new(GraphNodeKind::Topic, id)
    }

    pub fn file(id: i64) -> Result<Self> {
        Self::new(GraphNodeKind::File, id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphEdgeTrust {
    Trusted,
    DiagnosticHint,
}

impl GraphEdgeTrust {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::DiagnosticHint => "diagnostic_hint",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphEdgeType {
    Supersedes,
    Duplicates,
    Conflicts,
    DerivedFrom,
    MergedInto,
    SplitFrom,
    ExtractedFrom,
    Mentions,
    TouchesFile,
    HasState,
    HasTopic,
    SimilarTo,
    CandidateHint,
    CoOccursWith,
}

impl GraphEdgeType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supersedes => "supersedes",
            Self::Duplicates => "duplicates",
            Self::Conflicts => "conflicts",
            Self::DerivedFrom => "derived_from",
            Self::MergedInto => "merged_into",
            Self::SplitFrom => "split_from",
            Self::ExtractedFrom => "extracted_from",
            Self::Mentions => "mentions",
            Self::TouchesFile => "touches_file",
            Self::HasState => "has_state",
            Self::HasTopic => "has_topic",
            Self::SimilarTo => "similar_to",
            Self::CandidateHint => "candidate_hint",
            Self::CoOccursWith => "co_occurs_with",
        }
    }

    pub const fn trust(self) -> GraphEdgeTrust {
        match self {
            Self::SimilarTo | Self::CandidateHint | Self::CoOccursWith => {
                GraphEdgeTrust::DiagnosticHint
            }
            Self::Supersedes
            | Self::Duplicates
            | Self::Conflicts
            | Self::DerivedFrom
            | Self::MergedInto
            | Self::SplitFrom
            | Self::ExtractedFrom
            | Self::Mentions
            | Self::TouchesFile
            | Self::HasState
            | Self::HasTopic => GraphEdgeTrust::Trusted,
        }
    }

    pub const fn allows_endpoints(self, from: GraphNodeKind, to: GraphNodeKind) -> bool {
        match self {
            Self::Supersedes
            | Self::Duplicates
            | Self::Conflicts
            | Self::DerivedFrom
            | Self::MergedInto
            | Self::SplitFrom
            | Self::SimilarTo
            | Self::CandidateHint
            | Self::CoOccursWith => same_graph_node_kind(from, to),
            Self::ExtractedFrom => {
                matches!(
                    from,
                    GraphNodeKind::Entity
                        | GraphNodeKind::Fact
                        | GraphNodeKind::State
                        | GraphNodeKind::Topic
                ) && matches!(to, GraphNodeKind::Episode)
            }
            Self::Mentions => {
                matches!(from, GraphNodeKind::Memory | GraphNodeKind::Episode)
                    && matches!(to, GraphNodeKind::Entity)
            }
            Self::TouchesFile => {
                matches!(from, GraphNodeKind::Memory | GraphNodeKind::Episode)
                    && matches!(to, GraphNodeKind::File)
            }
            Self::HasState => {
                matches!(from, GraphNodeKind::Memory) && matches!(to, GraphNodeKind::State)
            }
            Self::HasTopic => {
                matches!(from, GraphNodeKind::Memory | GraphNodeKind::Episode)
                    && matches!(to, GraphNodeKind::Topic)
            }
        }
    }
}

const fn same_graph_node_kind(left: GraphNodeKind, right: GraphNodeKind) -> bool {
    matches!(
        (left, right),
        (GraphNodeKind::Memory, GraphNodeKind::Memory)
            | (GraphNodeKind::Entity, GraphNodeKind::Entity)
            | (GraphNodeKind::Fact, GraphNodeKind::Fact)
            | (GraphNodeKind::Episode, GraphNodeKind::Episode)
            | (GraphNodeKind::State, GraphNodeKind::State)
            | (GraphNodeKind::Topic, GraphNodeKind::Topic)
            | (GraphNodeKind::File, GraphNodeKind::File)
    )
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GraphEdgeProvenance<'a> {
    pub source_event_ids: &'a [i64],
    pub source_candidate_id: Option<i64>,
    pub source_operation_id: Option<i64>,
    pub confidence: Option<f64>,
    pub reason: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphEdgeInput<'a> {
    pub edge_type: GraphEdgeType,
    pub from_node: GraphNodeRef,
    pub to_node: GraphNodeRef,
    pub provenance: GraphEdgeProvenance<'a>,
    pub valid_from_epoch: Option<i64>,
    pub valid_to_epoch: Option<i64>,
}

pub fn insert_graph_edge(conn: &Connection, input: &GraphEdgeInput<'_>) -> Result<i64> {
    validate_graph_edge(conn, input)?;
    let now = chrono::Utc::now().timestamp();
    let source_event_ids = serde_json::to_string(input.provenance.source_event_ids)
        .context("serialize graph edge source event ids")?;
    conn.execute(
        "INSERT INTO graph_edges
         (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
          source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
          valid_from_epoch, valid_to_epoch, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            input.edge_type.as_str(),
            input.edge_type.trust().as_str(),
            input.from_node.kind.as_str(),
            input.from_node.id,
            input.to_node.kind.as_str(),
            input.to_node.id,
            source_event_ids,
            input.provenance.source_candidate_id,
            input.provenance.source_operation_id,
            input.provenance.confidence,
            input.provenance.reason,
            input.valid_from_epoch,
            input.valid_to_epoch,
            now
        ],
    )
    .context("insert graph edge")?;
    Ok(conn.last_insert_rowid())
}

fn validate_graph_edge(conn: &Connection, input: &GraphEdgeInput<'_>) -> Result<()> {
    if input.from_node == input.to_node {
        bail!("graph edge cannot link a node to itself");
    }
    if !input
        .edge_type
        .allows_endpoints(input.from_node.kind, input.to_node.kind)
    {
        bail!(
            "graph edge type {} does not allow {} to {} endpoints",
            input.edge_type.as_str(),
            input.from_node.kind.as_str(),
            input.to_node.kind.as_str()
        );
    }
    if let (Some(valid_from), Some(valid_to)) = (input.valid_from_epoch, input.valid_to_epoch) {
        if valid_to < valid_from {
            bail!("graph edge valid_to_epoch cannot be before valid_from_epoch");
        }
    }
    if let Some(confidence) = input.provenance.confidence {
        if !(0.0..=1.0).contains(&confidence) {
            bail!("graph edge confidence out of range");
        }
    }
    if input.edge_type.trust() == GraphEdgeTrust::Trusted {
        validate_trusted_provenance(conn, input.provenance)?;
    }
    Ok(())
}

fn validate_trusted_provenance(
    conn: &Connection,
    provenance: GraphEdgeProvenance<'_>,
) -> Result<()> {
    if provenance.source_event_ids.is_empty() {
        bail!("trusted graph edge requires source event ids");
    }
    if provenance.source_event_ids.iter().any(|id| *id <= 0) {
        bail!("trusted graph edge source event ids must be positive");
    }
    for event_id in provenance.source_event_ids {
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM captured_events WHERE id = ?1)",
                [event_id],
                |row| row.get(0),
            )
            .with_context(|| format!("validate graph edge source event id {event_id}"))?;
        if !exists {
            bail!("trusted graph edge source event id {event_id} does not exist");
        }
    }
    if provenance.source_candidate_id.is_none() {
        bail!("trusted graph edge requires source candidate id");
    }
    if provenance.source_operation_id.is_none() {
        bail!("trusted graph edge requires source operation id");
    }
    if provenance.confidence.is_none() {
        bail!("trusted graph edge requires confidence");
    }
    let reason = provenance.reason.unwrap_or_default().trim();
    if reason.is_empty() {
        bail!("trusted graph edge requires reason");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::facts::{insert_temporal_fact, FactPredicate, TemporalFactInput};

    struct GraphFixture {
        conn: Connection,
        memory_id: i64,
        second_memory_id: i64,
        entity_id: i64,
        second_entity_id: i64,
        fact_id: i64,
        episode_id: i64,
        state_id: i64,
        topic_id: i64,
        candidate_id: i64,
        operation_id: i64,
    }

    fn fixture() -> Result<GraphFixture> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;

        let now = 1_700_000_000_i64;
        let host_id: i64 =
            conn.query_row("SELECT id FROM hosts WHERE name = 'codex-cli'", [], |row| {
                row.get(0)
            })?;
        conn.execute(
            "INSERT INTO workspaces(root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch)
             VALUES ('/tmp/remem-graph', 'origin', 'main', ?1, ?1)",
            [now],
        )?;
        let workspace_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO projects(workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
             VALUES (?1, '/tmp/remem-graph', 'tmp-remem-graph', ?2, ?2)",
            params![workspace_id, now],
        )?;
        let project_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO sessions(host_id, workspace_id, project_id, session_id, started_at_epoch,
                                  last_seen_at_epoch, status)
             VALUES (?1, ?2, ?3, 'session-a', ?4, ?4, 'active')",
            params![host_id, workspace_id, project_id, now],
        )?;
        let session_row_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO captured_events(host_id, workspace_id, project_id, session_row_id,
                                         session_id, event_id, event_type, content_hash,
                                         retention_class, created_at_epoch, inserted_at_epoch)
             VALUES (?1, ?2, ?3, ?4, 'session-a', 'event-a', 'message',
                     'hash-a', 'default', ?5, ?5)",
            params![host_id, workspace_id, project_id, session_row_id, now],
        )?;
        let episode_id = conn.last_insert_rowid();

        let memory_id = crate::memory::insert_memory(
            &conn,
            Some("session-a"),
            "/tmp/remem-graph",
            Some("graph-contract"),
            "Graph contract",
            "Typed graph refs are available.",
            "decision",
            None,
        )?;
        let second_memory_id = crate::memory::insert_memory(
            &conn,
            Some("session-a"),
            "/tmp/remem-graph",
            Some("graph-contract-2"),
            "Graph contract follow-up",
            "Typed graph refs keep endpoints constrained.",
            "decision",
            None,
        )?;
        conn.execute(
            "INSERT INTO entities(canonical_name, entity_type, mention_count, created_at_epoch)
             VALUES ('Graph API', 'concept', 1, ?1)",
            [now],
        )?;
        let entity_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO entities(canonical_name, entity_type, mention_count, created_at_epoch)
             VALUES ('Graph API Review', 'concept', 1, ?1)",
            [now],
        )?;
        let second_entity_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memory_state_keys(owner_scope, owner_key, memory_type, state_key,
                                           state_label, current_memory_id,
                                           created_at_epoch, updated_at_epoch)
             VALUES ('project', '/tmp/remem-graph', 'decision', 'graph-contract',
                     'graph contract', ?1, ?2, ?2)",
            params![memory_id, now],
        )?;
        let state_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO topic_segments(host_id, project_id, session_row_id, project, topic_key,
                                        title, summary, status, segment_index,
                                        covered_from_event_id, covered_to_event_id,
                                        evidence_event_ids, confidence,
                                        created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, '/tmp/remem-graph', 'graph-contract',
                     'Graph contract', 'Typed graph refs.', 'resolved', 0,
                     ?4, ?4, ?5, 0.9, ?6, ?6)",
            params![
                host_id,
                project_id,
                session_row_id,
                episode_id,
                format!("[{episode_id}]"),
                now
            ],
        )?;
        let topic_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memory_candidates(project_id, scope, memory_type, topic_key, text,
                                           evidence_event_ids, confidence, risk_class,
                                           review_status, created_at_epoch, updated_at_epoch)
             VALUES (?1, 'project', 'decision', 'graph-contract', 'Typed graph refs.',
                     ?2, 0.9, 'low', 'accepted', ?3, ?3)",
            params![project_id, format!("[{episode_id}]"), now],
        )?;
        let candidate_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO memory_operation_log(operation, planner_version, actor, source,
                                             owner_scope, owner_key, memory_type, state_key,
                                             source_candidate_id, result_memory_id,
                                             superseded_ids, conflicting_ids, confidence,
                                             reason, created_at_epoch)
             VALUES ('add', 'graph-contract-test', 'test', 'memory_candidate',
                     'project', '/tmp/remem-graph', 'decision', 'graph-contract',
                     ?1, ?2, '[]', '[]', 0.9, 'test provenance', ?3)",
            params![candidate_id, memory_id, now],
        )?;
        let operation_id = conn.last_insert_rowid();
        let fact_id = insert_temporal_fact(
            &mut conn,
            &TemporalFactInput {
                project: "/tmp/remem-graph",
                subject: "graph-contract",
                predicate: FactPredicate::VerifiedBy,
                object: "cargo test memory::graph",
                valid_from_epoch: Some(now),
                valid_to_epoch: None,
                learned_at_epoch: Some(now),
                source_memory_id: Some(memory_id),
                source_observation_id: None,
                source_event_ids: &[episode_id],
                confidence: 0.9,
                supersedes_fact_id: None,
            },
        )?;

        Ok(GraphFixture {
            conn,
            memory_id,
            second_memory_id,
            entity_id,
            second_entity_id,
            fact_id,
            episode_id,
            state_id,
            topic_id,
            candidate_id,
            operation_id,
        })
    }

    fn trusted_provenance(fixture: &GraphFixture) -> GraphEdgeProvenance<'_> {
        GraphEdgeProvenance {
            source_event_ids: std::slice::from_ref(&fixture.episode_id),
            source_candidate_id: Some(fixture.candidate_id),
            source_operation_id: Some(fixture.operation_id),
            confidence: Some(0.9),
            reason: Some("test provenance"),
        }
    }

    fn insert_raw_trusted_mention(
        fixture: &GraphFixture,
        source_event_ids: &str,
    ) -> rusqlite::Result<usize> {
        fixture.conn.execute(
            "INSERT INTO graph_edges
             (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
              source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
              created_at_epoch)
             VALUES ('mentions', 'trusted', 'memory', ?1, 'entity', ?2,
                     ?3, ?4, ?5, 0.9, 'raw trusted provenance', 1)",
            params![
                fixture.memory_id,
                fixture.entity_id,
                source_event_ids,
                fixture.candidate_id,
                fixture.operation_id
            ],
        )
    }

    #[test]
    fn graph_node_refs_use_stable_db_values() -> Result<()> {
        assert_eq!(GraphNodeRef::memory(1)?.kind.as_str(), "memory");
        assert_eq!(GraphNodeRef::entity(1)?.kind.as_str(), "entity");
        assert_eq!(GraphNodeRef::fact(1)?.kind.as_str(), "fact");
        assert_eq!(GraphNodeRef::episode(1)?.kind.as_str(), "episode");
        assert_eq!(GraphNodeRef::state(1)?.kind.as_str(), "state");
        assert_eq!(GraphNodeRef::topic(1)?.kind.as_str(), "topic");
        assert!(GraphNodeRef::memory(0).is_err());
        Ok(())
    }

    #[test]
    fn trusted_edge_requires_complete_provenance() -> Result<()> {
        let fixture = fixture()?;
        let err = insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::Mentions,
                from_node: GraphNodeRef::memory(fixture.memory_id)?,
                to_node: GraphNodeRef::entity(fixture.entity_id)?,
                provenance: GraphEdgeProvenance::default(),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )
        .expect_err("trusted graph edge without provenance must fail");
        assert!(err.to_string().contains("source event ids"));
        Ok(())
    }

    #[test]
    fn inserts_typed_graph_edges_for_supported_node_refs() -> Result<()> {
        let fixture = fixture()?;
        let edges = [
            (
                GraphEdgeType::Mentions,
                GraphNodeRef::memory(fixture.memory_id)?,
                GraphNodeRef::entity(fixture.entity_id)?,
            ),
            (
                GraphEdgeType::ExtractedFrom,
                GraphNodeRef::fact(fixture.fact_id)?,
                GraphNodeRef::episode(fixture.episode_id)?,
            ),
            (
                GraphEdgeType::HasState,
                GraphNodeRef::memory(fixture.memory_id)?,
                GraphNodeRef::state(fixture.state_id)?,
            ),
            (
                GraphEdgeType::HasTopic,
                GraphNodeRef::memory(fixture.memory_id)?,
                GraphNodeRef::topic(fixture.topic_id)?,
            ),
        ];
        for (edge_type, from_node, to_node) in edges {
            let id = insert_graph_edge(
                &fixture.conn,
                &GraphEdgeInput {
                    edge_type,
                    from_node,
                    to_node,
                    provenance: trusted_provenance(&fixture),
                    valid_from_epoch: Some(1_700_000_000),
                    valid_to_epoch: None,
                },
            )?;
            assert!(id > 0);
        }

        let count: i64 = fixture
            .conn
            .query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
        assert_eq!(count, 4);
        Ok(())
    }

    #[test]
    fn diagnostic_hint_can_be_written_without_trusted_provenance() -> Result<()> {
        let fixture = fixture()?;
        let id = insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::SimilarTo,
                from_node: GraphNodeRef::memory(fixture.memory_id)?,
                to_node: GraphNodeRef::memory(fixture.second_memory_id)?,
                provenance: GraphEdgeProvenance::default(),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
        let trust: String = fixture.conn.query_row(
            "SELECT edge_trust FROM graph_edges WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        assert_eq!(trust, "diagnostic_hint");
        Ok(())
    }

    #[test]
    fn missing_target_node_fails_closed() -> Result<()> {
        let fixture = fixture()?;
        let err = insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::SimilarTo,
                from_node: GraphNodeRef::memory(fixture.memory_id)?,
                to_node: GraphNodeRef::memory(99_999)?,
                provenance: GraphEdgeProvenance::default(),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )
        .expect_err("missing graph target must fail");
        assert!(err.to_string().contains("insert graph edge"));
        let chain = format!("{err:#}");
        assert!(chain.contains("graph_edges to memory node missing"));
        Ok(())
    }

    #[test]
    fn trusted_edge_requires_existing_source_events() -> Result<()> {
        let fixture = fixture()?;
        let missing_event_id = 99_999_i64;
        let source_event_ids = [missing_event_id];
        let err = insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::Mentions,
                from_node: GraphNodeRef::memory(fixture.memory_id)?,
                to_node: GraphNodeRef::entity(fixture.entity_id)?,
                provenance: GraphEdgeProvenance {
                    source_event_ids: &source_event_ids,
                    source_candidate_id: Some(fixture.candidate_id),
                    source_operation_id: Some(fixture.operation_id),
                    confidence: Some(0.9),
                    reason: Some("missing event must fail"),
                },
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )
        .expect_err("trusted graph edge with missing evidence must fail");
        assert!(err
            .to_string()
            .contains("source event id 99999 does not exist"));
        Ok(())
    }

    #[test]
    fn invalid_edge_type_fails_closed_at_schema_boundary() -> Result<()> {
        let fixture = fixture()?;
        let err = fixture
            .conn
            .execute(
                "INSERT INTO graph_edges
                 (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
                  source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
                  created_at_epoch)
                 VALUES ('made_up', 'trusted', 'memory', ?1, 'entity', ?2,
                         ?3, ?4, ?5, 0.9, 'invalid edge type', 1)",
                params![
                    fixture.memory_id,
                    fixture.entity_id,
                    format!("[{}]", fixture.episode_id),
                    fixture.candidate_id,
                    fixture.operation_id
                ],
            )
            .expect_err("invalid graph edge type must fail");
        assert!(err.to_string().contains("CHECK constraint failed"));
        Ok(())
    }

    #[test]
    fn invalid_source_event_ids_fail_closed_at_schema_boundary() -> Result<()> {
        let fixture = fixture()?;
        for source_event_ids in ["[ ]", "not-json", "{\"id\":1}", "[\"event-a\"]", "[99999]"] {
            assert!(
                insert_raw_trusted_mention(&fixture, source_event_ids).is_err(),
                "invalid source_event_ids {source_event_ids:?} must fail"
            );
        }
        Ok(())
    }

    #[test]
    fn edge_type_rejects_invalid_endpoint_kinds() -> Result<()> {
        let fixture = fixture()?;
        let err = insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::HasState,
                from_node: GraphNodeRef::entity(fixture.entity_id)?,
                to_node: GraphNodeRef::memory(fixture.memory_id)?,
                provenance: trusted_provenance(&fixture),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )
        .expect_err("has_state with entity to memory endpoints must fail");
        assert!(err
            .to_string()
            .contains("graph edge type has_state does not allow entity to memory endpoints"));

        let err = fixture
            .conn
            .execute(
                "INSERT INTO graph_edges
                 (edge_type, edge_trust, from_node_kind, from_node_id, to_node_kind, to_node_id,
                  source_event_ids, source_candidate_id, source_operation_id, confidence, reason,
                  created_at_epoch)
                 VALUES ('has_state', 'trusted', 'entity', ?1, 'memory', ?2,
                         ?3, ?4, ?5, 0.9, 'invalid endpoints', 1)",
                params![
                    fixture.entity_id,
                    fixture.memory_id,
                    format!("[{}]", fixture.episode_id),
                    fixture.candidate_id,
                    fixture.operation_id
                ],
            )
            .expect_err("raw SQL invalid graph endpoints must fail");
        assert!(err.to_string().contains("CHECK constraint failed"));
        Ok(())
    }

    #[test]
    fn parent_node_deletes_remove_graph_edges() -> Result<()> {
        let fixture = fixture()?;
        let entity_edge_id = insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::Mentions,
                from_node: GraphNodeRef::memory(fixture.memory_id)?,
                to_node: GraphNodeRef::entity(fixture.second_entity_id)?,
                provenance: trusted_provenance(&fixture),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
        let state_edge_id = insert_graph_edge(
            &fixture.conn,
            &GraphEdgeInput {
                edge_type: GraphEdgeType::HasState,
                from_node: GraphNodeRef::memory(fixture.memory_id)?,
                to_node: GraphNodeRef::state(fixture.state_id)?,
                provenance: trusted_provenance(&fixture),
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )?;
        fixture.conn.execute(
            "DELETE FROM entities WHERE id = ?1",
            [fixture.second_entity_id],
        )?;
        let entity_edge_count: i64 = fixture.conn.query_row(
            "SELECT COUNT(*) FROM graph_edges WHERE id = ?1",
            [entity_edge_id],
            |row| row.get(0),
        )?;
        assert_eq!(entity_edge_count, 0);

        fixture.conn.execute(
            "DELETE FROM memory_state_keys WHERE id = ?1",
            [fixture.state_id],
        )?;
        let state_edge_count: i64 = fixture.conn.query_row(
            "SELECT COUNT(*) FROM graph_edges WHERE id = ?1",
            [state_edge_id],
            |row| row.get(0),
        )?;
        assert_eq!(state_edge_count, 0);
        Ok(())
    }
}
