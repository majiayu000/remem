use std::collections::BTreeSet;
use std::future::Future;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::memory::format::{xml_escape_attr, xml_escape_text};
use crate::memory::MemoryType;

mod apply;
mod auto_promote;
mod parse;
pub(crate) mod review;
pub(crate) mod review_stats;
mod route;
pub(crate) mod support;

use crate::runtime_config::SummaryGateMode;
use apply::{
    promote_candidate_to_memory_with_route, update_candidate_after_lifecycle, CandidateApplyOutcome,
};
pub(crate) use auto_promote::contains_unsafe_memory_marker;
use auto_promote::{candidate_promotion_decision, CandidatePromotionDecision};
use parse::{normalize_memory_type, normalize_scope, normalize_topic_key};
use parse::{parse_defer_reason, parse_memory_candidates};
pub(super) use route::{route_candidate, CandidateRoute};

const MEMORY_CANDIDATE_SYSTEM: &str = "\
Generate durable memory candidates from extracted observations.
Return zero or more <memory_candidate> blocks.
Each block must include <scope>, <type>, <topic_key>, <risk_class>, <confidence>, and <text>.
<type> must be one of the valid candidate memory types listed in the task. Observations use a different type vocabulary (feature/refactor/change are not candidate types), so never copy an observation's type verbatim into <type>; map feature/refactor/change to discovery.
Use scope=project unless the observation is explicitly a stable user preference.
Use risk_class=low only for factual project-scoped information that can be promoted without review.
If there is no durable memory candidate, return exactly <no_candidates reason=\"...\"/>.
If evidence is ambiguous or contradictory, return exactly <defer reason=\"...\"/> so it can be retried or reviewed.
Use only provided observations and evidence; do not invent files, outcomes, decisions, or facts.";

const SOURCE_KIND_OBSERVATION: &str = "observation";
const SOURCE_KIND_SUMMARY: &str = "summary";
const CANDIDATE_PROMPT_PREFERENCE_LIMIT: usize = 20;
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MemoryCandidateResult {
    EmptyRange,
    NoCandidates,
    Deferred {
        reason: String,
    },
    Written {
        candidates: usize,
        promoted: usize,
        pending_review: usize,
        to_event_id: i64,
    },
}

#[derive(Debug, Clone)]
struct SourceObservation {
    id: i64,
    observation_type: String,
    text: String,
    evidence_event_ids: Vec<i64>,
    confidence: f64,
}

struct ObservationBatch {
    from_event_id: i64,
    to_event_id: i64,
    evidence_event_ids: Vec<i64>,
    observations: Vec<SourceObservation>,
}

#[derive(Debug, Clone)]
struct CandidatePromptPreference {
    id: i64,
    text: String,
}

pub(crate) struct CandidatePromptObservation<'a> {
    pub(crate) id: i64,
    pub(crate) observation_type: &'a str,
    pub(crate) text: &'a str,
    pub(crate) evidence_event_ids: Vec<i64>,
    pub(crate) confidence: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedMemoryCandidate {
    pub(crate) scope: String,
    pub(crate) memory_type: String,
    pub(crate) topic_key: String,
    pub(crate) text: String,
    pub(crate) confidence: f64,
    pub(crate) risk_class: String,
}

pub(crate) fn parse_candidate_output(text: &str) -> Result<Vec<ParsedMemoryCandidate>> {
    parse_memory_candidates(text)
}

pub(crate) fn build_eval_candidate_request(
    project: &str,
    host: &str,
    session_id: Option<&str>,
    observations: &[CandidatePromptObservation<'_>],
) -> String {
    let mut evidence_set = BTreeSet::new();
    for observation in observations {
        for event_id in &observation.evidence_event_ids {
            evidence_set.insert(*event_id);
        }
    }
    let from_event_id = evidence_set.iter().copied().min().unwrap_or(0);
    let to_event_id = evidence_set.iter().copied().max().unwrap_or(0);
    let batch = ObservationBatch {
        from_event_id,
        to_event_id,
        evidence_event_ids: evidence_set.into_iter().collect(),
        observations: observations
            .iter()
            .map(|observation| SourceObservation {
                id: observation.id,
                observation_type: observation.observation_type.to_string(),
                text: observation.text.to_string(),
                evidence_event_ids: observation.evidence_event_ids.clone(),
                confidence: observation.confidence,
            })
            .collect(),
    };
    let task = eval_task(
        project,
        host,
        session_id,
        db::ExtractionTaskKind::MemoryCandidate,
    );
    format!(
        "{}\n\n<user_prompt>\n{}\n</user_prompt>",
        MEMORY_CANDIDATE_SYSTEM,
        build_candidate_prompt(&task, &batch, &[])
    )
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CandidatePersistSummary {
    pub(crate) candidates: usize,
    pub(crate) promoted: usize,
    pub(crate) pending_review: usize,
    pub(crate) summary_shadow_promoted: usize,
}

pub(crate) async fn process(task: &db::ExtractionTask) -> Result<MemoryCandidateResult> {
    let mut conn = db::open_db()?;
    let project = task.project.clone();
    let ai_profile = task.ai_profile.clone();
    process_with_generator(&mut conn, task, move |prompt| {
        let project = project.clone();
        let ai_profile = ai_profile.clone();
        async move {
            let profile = ai_profile.as_deref();
            crate::ai::call_ai(
                MEMORY_CANDIDATE_SYSTEM,
                &prompt,
                crate::ai::UsageContext {
                    project: Some(project.as_str()),
                    session_id: task.session_id.as_deref(),
                    operation: "memory_candidate",
                    host: profile.is_none().then_some(task.host.as_str()),
                    profile,
                },
            )
            .await
        }
    })
    .await
}

async fn process_with_generator<F, Fut>(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    generate: F,
) -> Result<MemoryCandidateResult>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let Some(batch) = load_observation_batch(conn, task)? else {
        return Ok(MemoryCandidateResult::EmptyRange);
    };

    let existing_preferences = load_candidate_prompt_preferences(conn, &task.project)?;
    let prompt = build_candidate_prompt(task, &batch, &existing_preferences);
    let response = generate(prompt).await?;
    let candidates = parse_memory_candidates(&response)?;
    if candidates.is_empty() {
        if let Some(reason) = parse_defer_reason(&response) {
            return Ok(MemoryCandidateResult::Deferred { reason });
        }
        if response.contains("<no_candidates") {
            enqueue_graph_followup(conn, task, batch.to_event_id)?;
            return Ok(MemoryCandidateResult::NoCandidates);
        }
        bail!("malformed memory_candidate output: no candidates parsed");
    }

    let result = persist_candidates(conn, task, &batch, &candidates)?;
    enqueue_graph_followup(conn, task, batch.to_event_id)?;
    crate::log::info(
        "memory-candidate",
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
    Ok(MemoryCandidateResult::Written {
        candidates: result.candidates,
        promoted: result.promoted,
        pending_review: result.pending_review,
        to_event_id: batch.to_event_id,
    })
}

fn load_candidate_prompt_preferences(
    conn: &Connection,
    project: &str,
) -> Result<Vec<CandidatePromptPreference>> {
    let preferences = crate::memory::preference::query_project_preferences(
        conn,
        project,
        CANDIDATE_PROMPT_PREFERENCE_LIMIT,
    )?;
    Ok(preferences
        .into_iter()
        .map(|preference| CandidatePromptPreference {
            id: preference.id,
            text: preference.text,
        })
        .collect())
}

fn enqueue_graph_followup(
    conn: &Connection,
    task: &db::ExtractionTask,
    high_watermark_event_id: i64,
) -> Result<()> {
    db::enqueue_followup_extraction_task(
        conn,
        task,
        db::ExtractionTaskKind::GraphCandidate,
        high_watermark_event_id,
    )?;
    Ok(())
}

fn load_observation_batch(
    conn: &Connection,
    task: &db::ExtractionTask,
) -> Result<Option<ObservationBatch>> {
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
        observations.push(SourceObservation {
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
    Ok(Some(ObservationBatch {
        from_event_id,
        to_event_id,
        evidence_event_ids: evidence_set.into_iter().collect(),
        observations,
    }))
}

fn persist_candidates(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    batch: &ObservationBatch,
    candidates: &[ParsedMemoryCandidate],
) -> Result<CandidatePersistSummary> {
    let source_texts = batch
        .observations
        .iter()
        .map(|observation| observation.text.as_str())
        .collect::<Vec<_>>();
    let route_texts = source_texts.clone();
    persist_candidate_rows(
        conn,
        CandidatePersistSource {
            project_id: task.project_id,
            project: &task.project,
            session_id: task.session_id.as_deref(),
            evidence_event_ids: &batch.evidence_event_ids,
            source_kind: SOURCE_KIND_OBSERVATION,
            summary_gate_mode: None,
            route_texts,
            source_texts,
        },
        candidates,
        Some(batch),
    )
}

pub(crate) fn persist_summary_candidates(
    conn: &mut Connection,
    session_id: &str,
    project_id: i64,
    project: &str,
    evidence_event_ids: &[i64],
    source_texts: &[String],
    candidates: &[ParsedMemoryCandidate],
) -> Result<CandidatePersistSummary> {
    let source_texts = source_texts.iter().map(String::as_str).collect::<Vec<_>>();
    let route_texts = source_texts.clone();
    persist_candidate_rows(
        conn,
        CandidatePersistSource {
            project_id,
            project,
            session_id: Some(session_id),
            evidence_event_ids,
            source_kind: SOURCE_KIND_SUMMARY,
            summary_gate_mode: Some(crate::runtime_config::summary_gate_mode()?),
            route_texts,
            source_texts,
        },
        candidates,
        None,
    )
}

#[cfg(test)]
pub(crate) fn persist_summary_candidates_with_gate_mode(
    conn: &mut Connection,
    session_id: &str,
    project_id: i64,
    project: &str,
    evidence_event_ids: &[i64],
    source_texts: &[String],
    candidates: &[ParsedMemoryCandidate],
    summary_gate_mode: SummaryGateMode,
) -> Result<CandidatePersistSummary> {
    let source_texts = source_texts.iter().map(String::as_str).collect::<Vec<_>>();
    let route_texts = source_texts.clone();
    persist_candidate_rows(
        conn,
        CandidatePersistSource {
            project_id,
            project,
            session_id: Some(session_id),
            evidence_event_ids,
            source_kind: SOURCE_KIND_SUMMARY,
            summary_gate_mode: Some(summary_gate_mode),
            route_texts,
            source_texts,
        },
        candidates,
        None,
    )
}

struct CandidatePersistSource<'a> {
    project_id: i64,
    project: &'a str,
    session_id: Option<&'a str>,
    evidence_event_ids: &'a [i64],
    source_kind: &'a str,
    summary_gate_mode: Option<SummaryGateMode>,
    route_texts: Vec<&'a str>,
    source_texts: Vec<&'a str>,
}

fn persist_candidate_rows(
    conn: &mut Connection,
    source: CandidatePersistSource<'_>,
    candidates: &[ParsedMemoryCandidate],
    auto_promote_batch: Option<&ObservationBatch>,
) -> Result<CandidatePersistSummary> {
    let evidence_json = serde_json::to_string(source.evidence_event_ids)?;
    let tx = conn.transaction()?;
    let mut summary = CandidatePersistSummary::default();
    for candidate in candidates {
        let now = chrono::Utc::now().timestamp();
        let (expires_at_epoch, valid_from_epoch) = crate::memory::lifecycle::ttl_metadata(
            &candidate.memory_type,
            Some(&candidate.topic_key),
            &candidate.text,
            now,
        );
        if candidate_exists(
            &tx,
            source.project_id,
            candidate,
            expires_at_epoch.is_some(),
            now,
        )? {
            continue;
        }

        let route = route_candidate(
            source.project,
            source.session_id,
            candidate,
            source.route_texts.iter().copied(),
        );
        let state_key = crate::memory::state_key::derive_state_key(
            &candidate.memory_type,
            Some(&candidate.topic_key),
            &candidate_title(candidate),
            &candidate.text,
        );
        let review_status = "pending_review";
        tx.execute(
            "INSERT INTO memory_candidates
             (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, created_at_epoch, updated_at_epoch,
              source_project, target_project, owner_scope, owner_key, topic_domain,
              routing_confidence, routing_reason, context_class, expires_at_epoch,
              valid_from_epoch, state_key, state_key_confidence, state_key_reason,
              source_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10,
                     ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,
                     ?21, ?22, ?23, ?24)",
            params![
                source.project_id,
                candidate.scope,
                candidate.memory_type,
                candidate.topic_key,
                candidate.text,
                evidence_json,
                candidate.confidence,
                candidate.risk_class,
                review_status,
                now,
                source.project,
                route.target_project.as_deref(),
                route.owner_scope,
                route.owner_key,
                route.topic_domain.as_deref(),
                route.routing_confidence,
                route.routing_reason,
                route.context_class,
                expires_at_epoch,
                valid_from_epoch,
                state_key
                    .as_ref()
                    .map(|decision| decision.state_key.as_str()),
                state_key.as_ref().map(|decision| decision.confidence),
                state_key.as_ref().map(|decision| decision.reason.as_str()),
                source.source_kind,
            ],
        )?;
        let candidate_id = tx.last_insert_rowid();
        summary.candidates += 1;

        match candidate_promotion_decision(
            candidate,
            auto_promote_batch,
            &route,
            &evidence_json,
            source.source_kind,
            source.summary_gate_mode,
            &source.source_texts,
        ) {
            CandidatePromotionDecision::Promote => {
                let outcome = promote_source_candidate(
                    &tx,
                    source.session_id,
                    source.project,
                    candidate_id,
                    candidate,
                    &evidence_json,
                    &route,
                )?;
                update_candidate_after_lifecycle(
                    &tx,
                    candidate_id,
                    candidate,
                    &route,
                    outcome.review_status_for("auto_promoted"),
                )?;
                if outcome.promoted {
                    summary.promoted += 1;
                }
            }
            CandidatePromotionDecision::PendingReview {
                block_reason,
                summary_shadow_promoted,
            } => {
                tx.execute(
                    "UPDATE memory_candidates SET auto_promote_block_reason = ?1 WHERE id = ?2",
                    params![block_reason, candidate_id],
                )?;
                if summary_shadow_promoted {
                    summary.summary_shadow_promoted += 1;
                    crate::log::info(
                        "memory-candidate",
                        &format!(
                            "summary gate shadow would promote: id={} type={} confidence={:.2}",
                            candidate_id, candidate.memory_type, candidate.confidence
                        ),
                    );
                }
                crate::log::warn(
                    "memory-candidate",
                    &format!(
                        "candidate routed to pending_review: id={} type={} scope={} risk={} confidence={:.2} reason={}",
                        candidate_id,
                        candidate.memory_type,
                        candidate.scope,
                        candidate.risk_class,
                        candidate.confidence,
                        block_reason
                    ),
                );
                summary.pending_review += 1;
            }
        }
    }
    tx.commit()?;
    Ok(summary)
}

fn candidate_exists(
    conn: &Connection,
    project_id: i64,
    candidate: &ParsedMemoryCandidate,
    candidate_has_ttl: bool,
    now_epoch: i64,
) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM memory_candidates
             WHERE project_id = ?1
               AND scope = ?2
               AND memory_type = ?3
               AND topic_key = ?4
               AND text = ?5
               AND (
                    ?6 = 0
                    OR (expires_at_epoch IS NOT NULL AND expires_at_epoch > ?7)
               )
             LIMIT 1",
            params![
                project_id,
                candidate.scope,
                candidate.memory_type,
                candidate.topic_key,
                candidate.text,
                if candidate_has_ttl { 1_i64 } else { 0_i64 },
                now_epoch
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(existing.is_some())
}

fn promote_source_candidate(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
    route: &CandidateRoute,
) -> Result<CandidateApplyOutcome> {
    promote_candidate_to_memory_with_route(
        conn,
        session_id,
        project,
        candidate_id,
        candidate,
        evidence_json,
        route,
    )
}

fn candidate_title(candidate: &ParsedMemoryCandidate) -> String {
    let first_line = candidate
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(&candidate.topic_key);
    crate::db::truncate_str(first_line, 96).to_string()
}

fn build_candidate_prompt(
    task: &db::ExtractionTask,
    batch: &ObservationBatch,
    existing_preferences: &[CandidatePromptPreference],
) -> String {
    let mut prompt = format!(
        "Task: memory_candidate\nProject: {}\nHost: {}\nSession: {}\nCovered evidence events: {}..{}\n\n",
        task.project,
        task.host,
        task.session_id.as_deref().unwrap_or("<unknown>"),
        batch.from_event_id,
        batch.to_event_id
    );
    append_existing_preferences(&mut prompt, existing_preferences);
    // 单一真实来源：从 MemoryType::ALL 动态生成合法 candidate type 列表注入 prompt，
    // 避免与枚举漂移（曾因 LLM 把 observation type feature/change 抄进 <type> 整批失败）。
    let valid_candidate_types = MemoryType::ALL
        .iter()
        .copied()
        .filter(|memory_type| *memory_type != MemoryType::SessionActivity)
        .map(|memory_type| memory_type.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    prompt.push_str(&format!(
        "Valid candidate <type> values: {valid_candidate_types}.\nDo not copy an observation's type verbatim; observations use a different vocabulary and feature/refactor/change must be mapped to discovery.\n\n"
    ));
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

fn append_existing_preferences(prompt: &mut String, preferences: &[CandidatePromptPreference]) {
    if preferences.is_empty() {
        return;
    }
    prompt.push_str("<existing_active_preferences>\n");
    prompt.push_str(
        "These preferences are already active for this project. Do not emit a new preference candidate that merely restates or paraphrases them; emit only net-new preferences, material refinements, or explicit contradictions supported by the observations.\n",
    );
    for preference in preferences {
        prompt.push_str(&format!(
            "<preference id=\"{}\">\n",
            xml_escape_attr(&preference.id.to_string())
        ));
        prompt.push_str(&xml_escape_text(&preference.text));
        prompt.push_str("\n</preference>\n");
    }
    prompt.push_str("</existing_active_preferences>\n\n");
}

fn eval_task(
    project: &str,
    host: &str,
    session_id: Option<&str>,
    task_kind: db::ExtractionTaskKind,
) -> db::ExtractionTask {
    db::ExtractionTask {
        id: 0,
        task_kind,
        host_id: 0,
        workspace_id: 0,
        project_id: 0,
        session_row_id: None,
        host: host.to_string(),
        project: project.to_string(),
        session_id: session_id.map(str::to_string),
        ai_profile: None,
        priority: 0,
        cursor_event_id: None,
        high_watermark_event_id: None,
        attempts: 0,
        replay_range_id: None,
    }
}

#[cfg(test)]
pub(super) mod tests;
#[cfg(test)]
mod tests_autopromote;
#[cfg(test)]
mod tests_autopromote_safety;
