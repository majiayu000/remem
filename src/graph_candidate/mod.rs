use std::collections::BTreeSet;
use std::future::Future;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::memory::graph_contract::{
    insert_graph_edge, GraphEdgeInput, GraphEdgeProvenance, GraphEdgeType, GraphNodeRef,
};
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::operation::{insert_operation_log, MemoryOperationInput, MemoryOperationPlan};

mod parser;
pub(crate) mod review;
mod source;

use parser::{parse_graph_candidates, parse_graph_defer_reason};
use source::{
    build_graph_candidate_prompt, graph_candidate_blocked_by_memory_candidates,
    graph_candidate_has_source_support, load_graph_observation_batch, GraphObservationBatch,
};

const GRAPH_CANDIDATE_SYSTEM: &str = "\
Generate governed graph candidates from extracted observations.
Return zero or more <graph_candidate> blocks.
Each block must include <type>, <edge_type>, <from_ref>, <to_ref>,
<evidence_event_ids>, <risk_class>, <confidence>, and <reason>.
Use type=edge for graph edges, type=entity_alias for alias/canonicalization,
type=claim for claim/fact candidates, and type=state_relation for current-state
relationships. Use only provided observations and evidence ids.
If there is no durable graph candidate, return exactly <no_graph_candidates reason=\"...\"/>.
If evidence is ambiguous or contradictory, return exactly <defer reason=\"...\"/>.
Do not invent files, entities, memories, evidence ids, or relationships.";

const AUTO_PROMOTE_MIN_CONFIDENCE: f64 = 0.85;
const REVIEW_APPROVAL_MIN_CONFIDENCE: f64 = 0.50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GraphCandidateResult {
    EmptyRange,
    NoCandidates,
    Deferred {
        reason: String,
    },
    Waiting {
        reason: String,
    },
    Written {
        candidates: usize,
        promoted: usize,
        pending_review: usize,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedGraphCandidate {
    pub(crate) candidate_type: String,
    pub(crate) edge_type: String,
    pub(crate) from_ref: String,
    pub(crate) to_ref: String,
    pub(crate) evidence_event_ids: Vec<i64>,
    pub(crate) confidence: f64,
    pub(crate) risk_class: String,
    pub(crate) reason: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct GraphCandidatePersistSummary {
    candidates: usize,
    promoted: usize,
    pending_review: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustedGraphEdgeOutcome {
    pub(crate) edge_id: i64,
    pub(crate) operation_id: i64,
}

pub(crate) async fn process_graph_candidate_task(
    task: &db::ExtractionTask,
) -> Result<GraphCandidateResult> {
    let mut conn = db::open_db()?;
    let project = task.project.clone();
    let ai_profile = task.ai_profile.clone();
    process_with_graph_generator(&mut conn, task, move |prompt| {
        let project = project.clone();
        let ai_profile = ai_profile.clone();
        async move {
            let profile = ai_profile.as_deref();
            crate::ai::call_ai(
                GRAPH_CANDIDATE_SYSTEM,
                &prompt,
                crate::ai::UsageContext {
                    project: Some(project.as_str()),
                    operation: "graph_candidate",
                    host: profile.is_none().then_some(task.host.as_str()),
                    profile,
                },
            )
            .await
        }
    })
    .await
}

async fn process_with_graph_generator<F, Fut>(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    generate: F,
) -> Result<GraphCandidateResult>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let Some(batch) = load_graph_observation_batch(conn, task)? else {
        return Ok(GraphCandidateResult::EmptyRange);
    };
    if let Some(reason) = graph_candidate_blocked_by_memory_candidates(conn, task, &batch)? {
        return Ok(GraphCandidateResult::Waiting { reason });
    }

    let prompt = build_graph_candidate_prompt(task, &batch);
    let response = generate(prompt).await?;
    let candidates = parse_graph_candidates(&response)?;
    if candidates.is_empty() {
        if let Some(reason) = parse_graph_defer_reason(&response) {
            return Ok(GraphCandidateResult::Deferred { reason });
        }
        if response.contains("<no_graph_candidates") {
            return Ok(GraphCandidateResult::NoCandidates);
        }
        bail!("malformed graph_candidate output: no candidates parsed");
    }

    let result = persist_graph_candidates(conn, task, &batch, &candidates)?;
    crate::log::info(
        "graph-candidate",
        &format!(
            "session={} range={}..{} candidates={} promoted={} pending_review={}",
            task.session_id.as_deref().unwrap_or("<unknown>"),
            batch.from_event_id,
            batch.to_event_id,
            result.candidates,
            result.promoted,
            result.pending_review
        ),
    );
    Ok(GraphCandidateResult::Written {
        candidates: result.candidates,
        promoted: result.promoted,
        pending_review: result.pending_review,
    })
}

fn persist_graph_candidates(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    batch: &GraphObservationBatch,
    candidates: &[ParsedGraphCandidate],
) -> Result<GraphCandidatePersistSummary> {
    let allowed_evidence = batch
        .evidence_event_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let tx = conn.transaction()?;
    let mut summary = GraphCandidatePersistSummary::default();
    for candidate in candidates {
        ensure_candidate_evidence(candidate, &allowed_evidence)?;
        let evidence_json = serde_json::to_string(&candidate.evidence_event_ids)?;
        if graph_candidate_exists(&tx, task.project_id, candidate, &evidence_json)? {
            continue;
        }

        let now = chrono::Utc::now().timestamp();
        tx.execute(
            "INSERT INTO graph_candidates
             (project_id, source_project, candidate_type, edge_type, from_ref, to_ref,
              evidence_event_ids, confidence, risk_class, reason, review_status,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     'pending_review', ?11, ?11)",
            params![
                task.project_id,
                task.project,
                candidate.candidate_type,
                candidate.edge_type,
                candidate.from_ref,
                candidate.to_ref,
                evidence_json,
                candidate.confidence,
                candidate.risk_class,
                candidate.reason,
                now
            ],
        )?;
        let candidate_id = tx.last_insert_rowid();
        summary.candidates += 1;

        let source_supported = graph_candidate_has_source_support(candidate, batch);
        let trusted_refs_valid =
            graph_candidate_has_trusted_refs(&tx, &task.project, task.project_id, candidate)?;
        if graph_should_auto_promote(candidate) && source_supported && trusted_refs_valid {
            let outcome = insert_trusted_graph_edge(
                &tx,
                &task.project,
                task.project_id,
                candidate_id,
                candidate,
                "graph_candidate",
            )?;
            mark_candidate_promoted(&tx, candidate_id, "auto_promoted", &outcome)?;
            summary.promoted += 1;
        } else {
            crate::log::warn(
                "graph-candidate",
                &format!(
                    "candidate routed to pending_review: id={} type={} edge={} risk={} confidence={:.2} reason={}",
                    candidate_id,
                    candidate.candidate_type,
                    candidate.edge_type,
                    candidate.risk_class,
                    candidate.confidence,
                    graph_auto_promote_block_reason(candidate, source_supported, trusted_refs_valid)
                ),
            );
            summary.pending_review += 1;
        }
    }
    tx.commit()?;
    Ok(summary)
}

fn ensure_candidate_evidence(
    candidate: &ParsedGraphCandidate,
    allowed_evidence: &BTreeSet<i64>,
) -> Result<()> {
    if candidate.evidence_event_ids.is_empty() {
        bail!("malformed graph_candidate output: missing evidence_event_ids");
    }
    for event_id in &candidate.evidence_event_ids {
        if !allowed_evidence.contains(event_id) {
            bail!("graph_candidate references evidence event id outside batch: {event_id}");
        }
    }
    Ok(())
}

fn graph_candidate_exists(
    conn: &Connection,
    project_id: i64,
    candidate: &ParsedGraphCandidate,
    evidence_json: &str,
) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM graph_candidates
             WHERE project_id = ?1
               AND candidate_type = ?2
               AND edge_type = ?3
               AND from_ref = ?4
               AND to_ref = ?5
               AND evidence_event_ids = ?6
             LIMIT 1",
            params![
                project_id,
                candidate.candidate_type,
                candidate.edge_type,
                candidate.from_ref,
                candidate.to_ref,
                evidence_json
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(existing.is_some())
}

pub(crate) fn insert_trusted_graph_edge(
    conn: &Connection,
    source_project: &str,
    project_id: i64,
    candidate_id: i64,
    candidate: &ParsedGraphCandidate,
    actor: &str,
) -> Result<TrustedGraphEdgeOutcome> {
    ensure_review_threshold(candidate)?;
    ensure_trusted_graph_refs(conn, source_project, candidate)?;
    let operation_input = MemoryOperationInput {
        source: "graph_candidate".to_string(),
        actor: actor.to_string(),
        source_project: source_project.to_string(),
        owner_scope: "repo".to_string(),
        owner_key: source_project.to_string(),
        memory_type: "graph_edge".to_string(),
        topic_key: Some(candidate.edge_type.clone()),
        state_key: None,
        source_candidate_id: Some(candidate_id),
        confidence: Some(candidate.confidence),
    };
    let operation_plan = MemoryOperationPlan::new(
        MemoryLifecycleOp::Add,
        None,
        "graph candidate promoted to trusted graph edge",
    );
    let operation_id = insert_operation_log(conn, &operation_input, &operation_plan, None)?;
    let edge_input = trusted_graph_edge_input(
        conn,
        source_project,
        project_id,
        candidate_id,
        operation_id,
        candidate,
    )?;
    let edge_id = insert_graph_edge(conn, &edge_input)?;
    Ok(TrustedGraphEdgeOutcome {
        edge_id,
        operation_id,
    })
}

pub(crate) fn mark_candidate_promoted(
    conn: &Connection,
    candidate_id: i64,
    status: &str,
    outcome: &TrustedGraphEdgeOutcome,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let updated = conn.execute(
        "UPDATE graph_candidates
         SET review_status = ?1,
             promoted_edge_id = ?2,
             source_operation_id = ?3,
             updated_at_epoch = ?4
         WHERE id = ?5
           AND review_status IN ('pending_review', 'deferred')",
        params![
            status,
            outcome.edge_id,
            outcome.operation_id,
            now,
            candidate_id
        ],
    )?;
    if updated == 0 {
        bail!(
            "graph candidate {candidate_id} is no longer reviewable, expected pending_review or deferred"
        );
    }
    Ok(())
}

fn graph_should_auto_promote(candidate: &ParsedGraphCandidate) -> bool {
    candidate.candidate_type == "edge"
        && matches!(candidate.edge_type.as_str(), "mentions" | "touches_file")
        && candidate.risk_class == "low"
        && candidate.confidence >= AUTO_PROMOTE_MIN_CONFIDENCE
        && !candidate.evidence_event_ids.is_empty()
        && (candidate.edge_type != "touches_file" || candidate.to_ref.starts_with("file:"))
}

fn ensure_review_threshold(candidate: &ParsedGraphCandidate) -> Result<()> {
    if candidate.evidence_event_ids.is_empty() {
        bail!("graph candidate approval requires explicit evidence_event_ids");
    }
    if candidate.confidence < REVIEW_APPROVAL_MIN_CONFIDENCE {
        bail!(
            "graph candidate confidence {:.2} is below review threshold {:.2}",
            candidate.confidence,
            REVIEW_APPROVAL_MIN_CONFIDENCE
        );
    }
    Ok(())
}

fn graph_candidate_has_trusted_refs(
    conn: &Connection,
    source_project: &str,
    project_id: i64,
    candidate: &ParsedGraphCandidate,
) -> Result<bool> {
    Ok(candidate_refs_resolve(conn, source_project, project_id, candidate).is_ok())
}

fn ensure_trusted_graph_refs(
    conn: &Connection,
    source_project: &str,
    candidate: &ParsedGraphCandidate,
) -> Result<()> {
    for reference in [&candidate.from_ref, &candidate.to_ref] {
        let Some(memory_id) = parse_memory_ref_id(reference)? else {
            continue;
        };
        if !active_repo_memory_exists(conn, source_project, memory_id)? {
            bail!(
                "graph candidate ref {reference} does not resolve to an active memory in {source_project}"
            );
        }
    }
    Ok(())
}

fn trusted_graph_edge_input<'a>(
    conn: &Connection,
    source_project: &str,
    project_id: i64,
    candidate_id: i64,
    operation_id: i64,
    candidate: &'a ParsedGraphCandidate,
) -> Result<GraphEdgeInput<'a>> {
    let edge_type = parse_trusted_graph_edge_type(&candidate.edge_type)?;
    let from_node = graph_node_ref(conn, source_project, project_id, &candidate.from_ref)?;
    let to_node = graph_node_ref(conn, source_project, project_id, &candidate.to_ref)?;
    Ok(GraphEdgeInput {
        edge_type,
        from_node,
        to_node,
        provenance: GraphEdgeProvenance {
            source_event_ids: &candidate.evidence_event_ids,
            source_candidate_id: Some(candidate_id),
            source_operation_id: Some(operation_id),
            confidence: Some(candidate.confidence),
            reason: Some(&candidate.reason),
        },
        valid_from_epoch: None,
        valid_to_epoch: None,
    })
}

fn candidate_refs_resolve(
    conn: &Connection,
    source_project: &str,
    project_id: i64,
    candidate: &ParsedGraphCandidate,
) -> Result<bool> {
    parse_trusted_graph_edge_type(&candidate.edge_type)?;
    candidate_ref_resolves(conn, source_project, project_id, &candidate.from_ref)?;
    candidate_ref_resolves(conn, source_project, project_id, &candidate.to_ref)?;
    Ok(true)
}

fn candidate_ref_resolves(
    conn: &Connection,
    source_project: &str,
    _project_id: i64,
    reference: &str,
) -> Result<()> {
    if let Some(memory_id) = parse_memory_ref_id(reference)? {
        if active_repo_memory_exists(conn, source_project, memory_id)? {
            return Ok(());
        }
        bail!(
            "graph candidate ref {reference} does not resolve to an active memory in {source_project}"
        );
    }
    if let Some(entity_name) = reference.strip_prefix("entity:") {
        resolve_entity_ref(conn, entity_name.trim())?;
        return Ok(());
    }
    if let Some(path) = reference.strip_prefix("file:") {
        if path.trim_start_matches("./").trim().is_empty() {
            bail!("graph file ref must not be empty");
        }
        return Ok(());
    }
    bail!("graph candidate ref {reference} is not supported by the typed graph contract")
}

fn parse_trusted_graph_edge_type(edge_type: &str) -> Result<GraphEdgeType> {
    match edge_type {
        "conflicts" => Ok(GraphEdgeType::Conflicts),
        "mentions" => Ok(GraphEdgeType::Mentions),
        "touches_file" => Ok(GraphEdgeType::TouchesFile),
        other => bail!("graph edge type {other} is not supported by the typed graph contract"),
    }
}

fn graph_node_ref(
    conn: &Connection,
    source_project: &str,
    project_id: i64,
    reference: &str,
) -> Result<GraphNodeRef> {
    if let Some(memory_id) = parse_memory_ref_id(reference)? {
        if !active_repo_memory_exists(conn, source_project, memory_id)? {
            bail!(
                "graph candidate ref {reference} does not resolve to an active memory in {source_project}"
            );
        }
        return GraphNodeRef::memory(memory_id);
    }
    if let Some(entity_name) = reference.strip_prefix("entity:") {
        let entity_id = resolve_entity_ref(conn, entity_name.trim())?;
        return GraphNodeRef::entity(entity_id);
    }
    if let Some(path) = reference.strip_prefix("file:") {
        let file_id = ensure_graph_file_node(conn, project_id, source_project, path.trim())?;
        return GraphNodeRef::file(file_id);
    }
    bail!("graph candidate ref {reference} is not supported by the typed graph contract")
}

fn resolve_entity_ref(conn: &Connection, entity_name: &str) -> Result<i64> {
    if entity_name.is_empty() {
        bail!("graph entity ref must not be empty");
    }
    let entity_id = conn
        .query_row(
            "SELECT id FROM entities WHERE lower(canonical_name) = lower(?1) LIMIT 1",
            [entity_name],
            |row| row.get(0),
        )
        .optional()?;
    entity_id.with_context(|| format!("graph entity ref entity:{entity_name} does not exist"))
}

fn ensure_graph_file_node(
    conn: &Connection,
    project_id: i64,
    source_project: &str,
    path: &str,
) -> Result<i64> {
    let normalized_path = path.trim_start_matches("./").trim();
    if normalized_path.is_empty() {
        bail!("graph file ref must not be empty");
    }
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO graph_file_nodes
         (project_id, source_project, path, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)
         ON CONFLICT(project_id, path) DO UPDATE
         SET source_project = excluded.source_project,
             updated_at_epoch = excluded.updated_at_epoch",
        params![project_id, source_project, normalized_path, now],
    )?;
    conn.query_row(
        "SELECT id FROM graph_file_nodes WHERE project_id = ?1 AND path = ?2",
        params![project_id, normalized_path],
        |row| row.get(0),
    )
    .context("load graph file node")
}

fn parse_memory_ref_id(reference: &str) -> Result<Option<i64>> {
    let Some(raw_id) = reference.strip_prefix("memory:") else {
        return Ok(None);
    };
    let memory_id = raw_id
        .trim()
        .parse::<i64>()
        .with_context(|| format!("graph candidate ref {reference} is not a numeric memory id"))?;
    if memory_id <= 0 {
        bail!("graph candidate ref {reference} must use a positive memory id");
    }
    Ok(Some(memory_id))
}

fn active_repo_memory_exists(
    conn: &Connection,
    source_project: &str,
    memory_id: i64,
) -> Result<bool> {
    conn.query_row(
        "SELECT 1
         FROM memories
         WHERE id = ?1
           AND status = 'active'
           AND (
                (owner_scope = 'repo' AND owner_key = ?2)
                OR target_project = ?2
                OR (
                    owner_scope IS NULL
                    AND project = ?2
                    AND COALESCE(scope, 'project') != 'global'
                )
           )
         LIMIT 1",
        params![memory_id, source_project],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
    .map_err(Into::into)
}

fn graph_auto_promote_block_reason(
    candidate: &ParsedGraphCandidate,
    source_supported: bool,
    trusted_refs_valid: bool,
) -> &'static str {
    if candidate.candidate_type != "edge" {
        return "candidate_type_requires_review";
    }
    if !matches!(candidate.edge_type.as_str(), "mentions" | "touches_file") {
        return "edge_type_requires_review";
    }
    if candidate.risk_class != "low" {
        return "risk_class_not_low";
    }
    if candidate.confidence < AUTO_PROMOTE_MIN_CONFIDENCE {
        return "confidence_below_threshold";
    }
    if candidate.evidence_event_ids.is_empty() {
        return "missing_evidence_ids";
    }
    if candidate.edge_type == "touches_file" && !candidate.to_ref.starts_with("file:") {
        return "touches_file_target_not_file_ref";
    }
    if !source_supported {
        return "source_observation_missing_ref_support";
    }
    if !trusted_refs_valid {
        return "trusted_ref_unresolved";
    }
    "unknown"
}

#[cfg(test)]
mod tests;
