use std::collections::BTreeSet;
use std::future::Future;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::memory::format::{xml_escape_attr, xml_escape_text};
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::operation::{insert_operation_log, MemoryOperationInput, MemoryOperationPlan};

mod parser;
pub(crate) mod review;

use parser::{parse_graph_candidates, parse_graph_defer_reason};

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

#[derive(Debug, Clone)]
struct GraphSourceObservation {
    id: i64,
    observation_type: String,
    text: String,
    evidence_event_ids: Vec<i64>,
    confidence: f64,
}

struct GraphObservationBatch {
    from_event_id: i64,
    to_event_id: i64,
    evidence_event_ids: Vec<i64>,
    observations: Vec<GraphSourceObservation>,
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

fn load_graph_observation_batch(
    conn: &Connection,
    task: &db::ExtractionTask,
) -> Result<Option<GraphObservationBatch>> {
    let Some(session_row_id) = task.session_row_id else {
        return Ok(None);
    };
    let Some(high_watermark) = task.high_watermark_event_id else {
        return Ok(None);
    };
    let cursor = task.cursor_event_id.unwrap_or(0);
    if high_watermark <= cursor {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT id,
                COALESCE(observation_type, type, 'discovery') AS observation_type,
                COALESCE(text, narrative, title, '') AS text,
                evidence_event_ids,
                COALESCE(confidence, 0.5) AS confidence
         FROM observations
         WHERE session_row_id = ?1
           AND evidence_event_ids IS NOT NULL
           AND text IS NOT NULL
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map(params![session_row_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, f64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut observations = Vec::new();
    let mut evidence_set = BTreeSet::new();
    for (id, observation_type, text, evidence_json, confidence) in rows {
        let event_ids: Vec<i64> = serde_json::from_str(&evidence_json)
            .with_context(|| format!("observation {id} has malformed evidence_event_ids"))?;
        let in_range = event_ids
            .iter()
            .any(|event_id| *event_id > cursor && *event_id <= high_watermark);
        if !in_range {
            continue;
        }
        for event_id in &event_ids {
            evidence_set.insert(*event_id);
        }
        observations.push(GraphSourceObservation {
            id,
            observation_type,
            text,
            evidence_event_ids: event_ids,
            confidence,
        });
    }

    if observations.is_empty() || evidence_set.is_empty() {
        return Ok(None);
    }
    let from_event_id = *evidence_set.iter().next().unwrap_or(&0);
    let to_event_id = *evidence_set.iter().next_back().unwrap_or(&0);
    Ok(Some(GraphObservationBatch {
        from_event_id,
        to_event_id,
        evidence_event_ids: evidence_set.into_iter().collect(),
        observations,
    }))
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
        if graph_should_auto_promote(candidate) && source_supported {
            let outcome = insert_trusted_graph_edge(
                &tx,
                &task.project,
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
                    graph_auto_promote_block_reason(candidate, source_supported)
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

fn graph_candidate_has_source_support(
    candidate: &ParsedGraphCandidate,
    batch: &GraphObservationBatch,
) -> bool {
    let candidate_evidence = candidate
        .evidence_event_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    batch
        .observations
        .iter()
        .filter(|observation| {
            observation
                .evidence_event_ids
                .iter()
                .any(|event_id| candidate_evidence.contains(event_id))
        })
        .any(|observation| {
            ref_supported_by_text(&candidate.from_ref, &observation.text)
                && ref_supported_by_text(&candidate.to_ref, &observation.text)
        })
}

fn ref_supported_by_text(reference: &str, text: &str) -> bool {
    let haystack = text.to_ascii_lowercase();
    reference_support_needles(reference)
        .into_iter()
        .any(|needle| contains_support_needle(&haystack, &needle))
}

fn reference_support_needles(reference: &str) -> Vec<String> {
    let reference = reference.trim();
    if let Some(path) = reference.strip_prefix("file:") {
        return vec![path.trim_start_matches("./").to_string(), path.to_string()];
    }
    if let Some(entity) = reference.strip_prefix("entity:") {
        return vec![entity.to_string(), entity.replace(['_', '-'], " ")];
    }
    if let Some(memory_id) = reference.strip_prefix("memory:") {
        return vec![reference.to_string(), format!("memory {memory_id}")];
    }
    vec![reference.to_string()]
}

fn contains_support_needle(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim().to_ascii_lowercase();
    !needle.is_empty() && haystack.contains(&needle)
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
    candidate_id: i64,
    candidate: &ParsedGraphCandidate,
    actor: &str,
) -> Result<TrustedGraphEdgeOutcome> {
    ensure_review_threshold(candidate)?;
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
    let evidence_json = serde_json::to_string(&candidate.evidence_event_ids)?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO graph_edges
         (edge_type, from_ref, to_ref, source_candidate_id, evidence_event_ids,
          source_operation_id, confidence, reason, created_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            candidate.edge_type,
            candidate.from_ref,
            candidate.to_ref,
            candidate_id,
            evidence_json,
            operation_id,
            candidate.confidence,
            candidate.reason,
            now
        ],
    )?;
    Ok(TrustedGraphEdgeOutcome {
        edge_id: conn.last_insert_rowid(),
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
           AND review_status = 'pending_review'",
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
            "graph candidate {candidate_id} is no longer pending_review, expected pending_review"
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

fn graph_auto_promote_block_reason(
    candidate: &ParsedGraphCandidate,
    source_supported: bool,
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
    "unknown"
}

fn build_graph_candidate_prompt(
    task: &db::ExtractionTask,
    batch: &GraphObservationBatch,
) -> String {
    let mut prompt = format!(
        "Task: graph_candidate\nProject: {}\nHost: {}\nSession: {}\nCovered evidence events: {}..{}\n\n",
        task.project,
        task.host,
        task.session_id.as_deref().unwrap_or("<unknown>"),
        batch.from_event_id,
        batch.to_event_id
    );
    for observation in &batch.observations {
        let evidence = observation
            .evidence_event_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        prompt.push_str(&format!(
            "<observation id=\"{}\" type=\"{}\" confidence=\"{}\" evidence_event_ids=\"{}\">\n",
            observation.id,
            xml_escape_attr(&observation.observation_type),
            observation.confidence,
            xml_escape_attr(&evidence)
        ));
        prompt.push_str(&xml_escape_text(&observation.text));
        prompt.push_str("\n</observation>\n\n");
    }
    prompt
}

#[cfg(test)]
mod tests;
