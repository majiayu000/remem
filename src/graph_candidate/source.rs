use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::graph_candidate::ParsedGraphCandidate;
use crate::memory::format::{xml_escape_attr, xml_escape_text};

#[derive(Debug, Clone)]
struct GraphSourceObservation {
    id: i64,
    observation_type: String,
    text: String,
    files_read: Vec<String>,
    files_modified: Vec<String>,
    evidence_event_ids: Vec<i64>,
    confidence: f64,
}

#[derive(Debug, Clone)]
struct GraphSourceEvent {
    id: i64,
    text: String,
}

pub(super) struct GraphObservationBatch {
    pub(super) from_event_id: i64,
    pub(super) to_event_id: i64,
    pub(super) evidence_event_ids: Vec<i64>,
    source_events: Vec<GraphSourceEvent>,
    observations: Vec<GraphSourceObservation>,
}

pub(super) fn load_graph_observation_batch(
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
                files_read,
                files_modified,
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
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, f64>(6)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut observations = Vec::new();
    let mut evidence_set = BTreeSet::new();
    for (
        id,
        observation_type,
        text,
        files_read_json,
        files_modified_json,
        evidence_json,
        confidence,
    ) in rows
    {
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
            files_read: parse_observation_file_list(id, "files_read", files_read_json)?,
            files_modified: parse_observation_file_list(id, "files_modified", files_modified_json)?,
            evidence_event_ids: event_ids,
            confidence,
        });
    }

    if observations.is_empty() || evidence_set.is_empty() {
        return Ok(None);
    }
    let from_event_id = *evidence_set.iter().next().unwrap_or(&0);
    let to_event_id = *evidence_set.iter().next_back().unwrap_or(&0);
    let evidence_event_ids = evidence_set.into_iter().collect::<Vec<_>>();
    let source_events = load_graph_source_events(conn, &evidence_event_ids)?;
    Ok(Some(GraphObservationBatch {
        from_event_id,
        to_event_id,
        evidence_event_ids,
        source_events,
        observations,
    }))
}

fn load_graph_source_events(
    conn: &Connection,
    evidence_event_ids: &[i64],
) -> Result<Vec<GraphSourceEvent>> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.id = ?1",
    )?;
    let mut events = Vec::new();
    for event_id in evidence_event_ids {
        if let Some(text) = stmt
            .query_row(params![event_id], |row| row.get(0))
            .optional()?
        {
            events.push(GraphSourceEvent {
                id: *event_id,
                text,
            });
        }
    }
    Ok(events)
}

fn parse_observation_file_list(
    observation_id: i64,
    field: &str,
    raw: Option<String>,
) -> Result<Vec<String>> {
    let Some(raw) = raw.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<String>>(raw)
        .with_context(|| format!("observation {observation_id} has malformed {field}"))
}

pub(super) fn graph_candidate_blocked_by_memory_candidates(
    conn: &Connection,
    task: &db::ExtractionTask,
    batch: &GraphObservationBatch,
) -> Result<Option<String>> {
    let Some(session_row_id) = task.session_row_id else {
        return Ok(None);
    };
    let high_watermark = task.high_watermark_event_id.unwrap_or(batch.to_event_id);
    let incomplete_memory_task: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, status
             FROM extraction_tasks
             WHERE host_id = ?1
               AND project_id = ?2
               AND session_row_id = ?3
               AND task_kind = 'memory_candidate'
               AND COALESCE(high_watermark_event_id, 0) >= ?4
               AND status != 'done'
             ORDER BY id ASC
             LIMIT 1",
            params![
                task.host_id,
                task.project_id,
                session_row_id,
                high_watermark
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((task_id, status)) = incomplete_memory_task {
        return Ok(Some(format!(
            "memory_candidate task {task_id} is {status}; graph extraction waits for memory extraction completion"
        )));
    }

    let evidence_json = serde_json::to_string(&batch.evidence_event_ids)?;
    let pending_memory_candidates: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memory_candidates
         WHERE project_id = ?1
           AND evidence_event_ids = ?2
           AND review_status = 'pending_review'",
        params![task.project_id, evidence_json],
        |row| row.get(0),
    )?;
    if pending_memory_candidates > 0 {
        return Ok(Some(format!(
            "{pending_memory_candidates} memory candidate(s) for this evidence range are pending review"
        )));
    }

    Ok(None)
}

pub(super) fn graph_candidate_has_source_support(
    candidate: &ParsedGraphCandidate,
    batch: &GraphObservationBatch,
) -> bool {
    let candidate_evidence = candidate
        .evidence_event_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let observation_supported = batch
        .observations
        .iter()
        .filter(|observation| {
            observation
                .evidence_event_ids
                .iter()
                .any(|event_id| candidate_evidence.contains(event_id))
        })
        .any(|observation| refs_supported_by_observation(candidate, observation));
    let cited_events_supported = candidate.evidence_event_ids.iter().all(|event_id| {
        batch
            .source_events
            .iter()
            .find(|event| event.id == *event_id)
            .is_some_and(|event| refs_supported_by_text(candidate, &event.text))
    });
    observation_supported && cited_events_supported
}

fn refs_supported_by_observation(
    candidate: &ParsedGraphCandidate,
    observation: &GraphSourceObservation,
) -> bool {
    refs_supported_by_text(candidate, &observation_support_text(observation))
}

fn observation_support_text(observation: &GraphSourceObservation) -> String {
    let mut text = observation.text.clone();
    for file in observation
        .files_read
        .iter()
        .chain(observation.files_modified.iter())
    {
        text.push('\n');
        text.push_str(file);
    }
    text
}

fn refs_supported_by_text(candidate: &ParsedGraphCandidate, text: &str) -> bool {
    ref_supported_by_text(&candidate.from_ref, text)
        && ref_supported_by_text(&candidate.to_ref, text)
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

pub(super) fn build_graph_candidate_prompt(
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
        if !observation.files_read.is_empty() {
            prompt.push_str("\n<files_read>\n");
            for file in &observation.files_read {
                prompt.push_str(&xml_escape_text(file));
                prompt.push('\n');
            }
            prompt.push_str("</files_read>\n");
        }
        if !observation.files_modified.is_empty() {
            prompt.push_str("\n<files_modified>\n");
            for file in &observation.files_modified {
                prompt.push_str(&xml_escape_text(file));
                prompt.push('\n');
            }
            prompt.push_str("</files_modified>\n");
        }
        prompt.push_str("\n</observation>\n\n");
    }
    prompt
}
