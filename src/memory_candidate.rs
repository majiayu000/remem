use std::collections::BTreeSet;
use std::future::Future;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::memory::format::{xml_escape_attr, xml_escape_text};
use crate::memory::MemoryType;

mod parse;
pub(crate) mod review;

use parse::{normalize_memory_type, normalize_scope, normalize_topic_key};
use parse::{parse_defer_reason, parse_memory_candidates};

const MEMORY_CANDIDATE_SYSTEM: &str = "\
Generate durable memory candidates from extracted observations.
Return zero or more <memory_candidate> blocks.
Each block must include <scope>, <type>, <topic_key>, <risk_class>, <confidence>, and <text>.
Use scope=project unless the observation is explicitly a stable user preference.
Use risk_class=low only for factual project-scoped information that can be promoted without review.
If there is no durable memory candidate, return exactly <no_candidates reason=\"...\"/>.
If evidence is ambiguous or contradictory, return exactly <defer reason=\"...\"/> so it can be retried or reviewed.
Use only provided observations and evidence; do not invent files, outcomes, decisions, or facts.";

const AUTO_PROMOTE_MIN_CONFIDENCE: f64 = 0.80;
const AUTO_PROMOTE_MIN_OBSERVATION_CONFIDENCE: f64 = 0.75;
const AUTO_PROMOTE_UNSAFE_MARKERS: &[&str] = &[
    "api key",
    "apikey",
    "authorization:",
    "bearer ",
    "credential",
    "credit card",
    "password",
    "payment",
    "private key",
    "secret",
    "sk-",
    "token",
];

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

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ParsedMemoryCandidate {
    pub(super) scope: String,
    pub(super) memory_type: String,
    pub(super) topic_key: String,
    pub(super) text: String,
    pub(super) confidence: f64,
    pub(super) risk_class: String,
}

pub(crate) async fn process(task: &db::ExtractionTask) -> Result<MemoryCandidateResult> {
    let mut conn = db::open_db()?;
    let project = task.project.clone();
    process_with_generator(&mut conn, task, move |prompt| {
        let project = project.clone();
        async move {
            crate::ai::call_ai(
                MEMORY_CANDIDATE_SYSTEM,
                &prompt,
                crate::ai::UsageContext {
                    project: Some(project.as_str()),
                    operation: "memory_candidate",
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

    let prompt = build_candidate_prompt(task, &batch);
    let response = generate(prompt).await?;
    let candidates = parse_memory_candidates(&response)?;
    if candidates.is_empty() {
        if let Some(reason) = parse_defer_reason(&response) {
            return Ok(MemoryCandidateResult::Deferred { reason });
        }
        if response.contains("<no_candidates") {
            return Ok(MemoryCandidateResult::NoCandidates);
        }
        bail!("malformed memory_candidate output: no candidates parsed");
    }

    let result = persist_candidates(conn, task, &batch, &candidates)?;
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
    })
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
) -> Result<PersistSummary> {
    let evidence_json = serde_json::to_string(&batch.evidence_event_ids)?;
    let tx = conn.transaction()?;
    let mut summary = PersistSummary::default();
    for candidate in candidates {
        if candidate_exists(&tx, task.project_id, candidate)? {
            continue;
        }

        let review_status = if should_auto_promote(candidate, batch) {
            "auto_promoted"
        } else {
            "pending_review"
        };
        let now = chrono::Utc::now().timestamp();
        tx.execute(
            "INSERT INTO memory_candidates
             (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                task.project_id,
                candidate.scope,
                candidate.memory_type,
                candidate.topic_key,
                candidate.text,
                evidence_json,
                candidate.confidence,
                candidate.risk_class,
                review_status,
                now
            ],
        )?;
        let candidate_id = tx.last_insert_rowid();
        summary.candidates += 1;

        if review_status == "auto_promoted" {
            promote_task_candidate(&tx, task, candidate_id, candidate, &evidence_json)?;
            summary.promoted += 1;
        } else {
            summary.pending_review += 1;
        }
    }
    tx.commit()?;
    Ok(summary)
}

#[derive(Default)]
struct PersistSummary {
    candidates: usize,
    promoted: usize,
    pending_review: usize,
}

fn candidate_exists(
    conn: &Connection,
    project_id: i64,
    candidate: &ParsedMemoryCandidate,
) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM memory_candidates
             WHERE project_id = ?1
               AND scope = ?2
               AND memory_type = ?3
               AND topic_key = ?4
               AND text = ?5
             LIMIT 1",
            params![
                project_id,
                candidate.scope,
                candidate.memory_type,
                candidate.topic_key,
                candidate.text
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(existing.is_some())
}

fn promote_task_candidate(
    conn: &Connection,
    task: &db::ExtractionTask,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
) -> Result<()> {
    promote_candidate_to_memory(
        conn,
        task.session_id.as_deref(),
        &task.project,
        candidate_id,
        candidate,
        evidence_json,
    )?;
    Ok(())
}

pub(super) fn promote_candidate_to_memory(
    conn: &Connection,
    session_id: Option<&str>,
    project: &str,
    candidate_id: i64,
    candidate: &ParsedMemoryCandidate,
    evidence_json: &str,
) -> Result<i64> {
    let title = candidate_title(candidate);
    let memory_id = if candidate.memory_type == "lesson" {
        crate::memory::lesson::save_lesson(
            conn,
            &crate::memory::lesson::SaveLessonRequest {
                session_id,
                project,
                topic_key: Some(&candidate.topic_key),
                title: &title,
                content: &candidate.text,
                confidence: candidate.confidence,
                source_evidence: Some(evidence_json),
                files: None,
                branch: None,
                scope: &candidate.scope,
                created_at_epoch: None,
                stale_after_epoch: None,
            },
        )?
    } else {
        crate::memory::insert_memory_full(
            conn,
            session_id,
            project,
            Some(&candidate.topic_key),
            &title,
            &candidate.text,
            &candidate.memory_type,
            None,
            None,
            &candidate.scope,
            None,
        )?
    };
    conn.execute(
        "UPDATE memories
         SET evidence_event_ids = ?1,
             source_candidate_id = ?2,
             confidence = ?3
        WHERE id = ?4",
        params![evidence_json, candidate_id, candidate.confidence, memory_id],
    )?;
    Ok(memory_id)
}

fn should_auto_promote(candidate: &ParsedMemoryCandidate, batch: &ObservationBatch) -> bool {
    candidate.scope == "project"
        && candidate.risk_class == "low"
        && candidate.confidence >= AUTO_PROMOTE_MIN_CONFIDENCE
        && MemoryType::parse(&candidate.memory_type).is_some_and(MemoryType::auto_promote)
        && !contains_auto_promote_unsafe_marker(&candidate.text)
        && is_supported_by_source_observation(candidate, batch)
}

fn is_supported_by_source_observation(
    candidate: &ParsedMemoryCandidate,
    batch: &ObservationBatch,
) -> bool {
    let candidate_text = normalize_evidence_text(&candidate.text);
    if candidate_text.chars().count() < 24 {
        return false;
    }
    batch.observations.iter().any(|observation| {
        observation.confidence >= AUTO_PROMOTE_MIN_OBSERVATION_CONFIDENCE
            && observation.observation_type == candidate.memory_type
            && normalize_evidence_text(&observation.text).contains(&candidate_text)
    })
}

fn contains_auto_promote_unsafe_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    AUTO_PROMOTE_UNSAFE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn normalize_evidence_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
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

fn build_candidate_prompt(task: &db::ExtractionTask, batch: &ObservationBatch) -> String {
    let mut prompt = format!(
        "Task: memory_candidate\nProject: {}\nHost: {}\nSession: {}\nCovered evidence events: {}..{}\n\n",
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
