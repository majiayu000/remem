use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::poisoning::{
    event_trust_class, scan_source_instruction_pattern, SourceTrustClass,
};

use super::support::supporting_source_groups;
use super::ParsedMemoryCandidate;

#[derive(Debug)]
pub(super) struct SummaryCandidateEvidence {
    pub(super) event_ids: Option<Vec<i64>>,
    pub(super) source_texts: Vec<String>,
}

pub(super) struct SummaryEvidenceResolver {
    events: BTreeMap<i64, CapturedSourceEvent>,
}

#[derive(Debug)]
struct CapturedSourceEvent {
    id: i64,
    text: String,
    trust: SourceTrustClass,
}

impl SummaryEvidenceResolver {
    pub(super) fn load(
        conn: &Connection,
        evidence_event_ids: &[i64],
        _source_kind: &str,
    ) -> Result<Self> {
        Ok(Self {
            events: load_captured_source_events(conn, evidence_event_ids)?,
        })
    }

    pub(super) fn resolve(&self, candidate: &ParsedMemoryCandidate) -> SummaryCandidateEvidence {
        let events = self.events.values().collect::<Vec<_>>();
        let Some(selected_ids) = bind_candidate_to_events(candidate, &events) else {
            return SummaryCandidateEvidence {
                event_ids: None,
                source_texts: self
                    .events
                    .values()
                    .map(|event| event.text.clone())
                    .collect(),
            };
        };
        let source_texts = self
            .events
            .values()
            .filter(|event| selected_ids.contains(&event.id))
            .map(|event| event.text.clone())
            .collect();
        SummaryCandidateEvidence {
            event_ids: Some(selected_ids.into_iter().collect()),
            source_texts,
        }
    }

    /// Reuse a batch's loaded sources without letting another candidate's
    /// events authorize this one. Every recorded event must still be present,
    /// safe, and part of the deterministic per-claim support binding.
    pub(super) fn proof_sources<'a>(
        &'a self,
        candidate: &ParsedMemoryCandidate,
        event_ids: &[i64],
    ) -> Option<(Vec<&'a str>, SourceTrustClass)> {
        let events = event_ids
            .iter()
            .map(|id| self.events.get(id))
            .collect::<Option<Vec<_>>>()?;
        if events.is_empty()
            || events
                .iter()
                .any(|event| scan_source_instruction_pattern(&event.text).is_some())
            || bind_candidate_to_events(candidate, &events)? != event_ids.iter().copied().collect()
        {
            return None;
        }
        let trust = events.iter().map(|event| event.trust).min()?;
        Some((
            events.iter().map(|event| event.text.as_str()).collect(),
            trust,
        ))
    }
}

fn bind_candidate_to_events(
    candidate: &ParsedMemoryCandidate,
    events: &[&CapturedSourceEvent],
) -> Option<BTreeSet<i64>> {
    let source_texts = events
        .iter()
        .map(|event| event.text.as_str())
        .collect::<Vec<_>>();
    let groups = supporting_source_groups(&candidate.text, &source_texts)?;

    let mut selected_ids = BTreeSet::new();
    for group in groups {
        let mut best: Option<&CapturedSourceEvent> = None;
        for source_index in group {
            let Some(&event) = events.get(source_index) else {
                continue;
            };
            let should_replace = best.is_none_or(|current| {
                event.trust > current.trust
                    || (event.trust == current.trust && event.id < current.id)
            });
            if should_replace {
                best = Some(event);
            }
        }
        selected_ids.insert(best?.id);
    }
    Some(selected_ids)
}

fn load_captured_source_events(
    conn: &Connection,
    evidence_event_ids: &[i64],
) -> Result<BTreeMap<i64, CapturedSourceEvent>> {
    if evidence_event_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut stmt = conn.prepare(
        "SELECT e.id, e.event_type, e.role, e.tool_name, e.content_text,
                COALESCE(
                    CASE
                        WHEN b.content_encoding = 'plain' THEN CAST(b.content_bytes AS TEXT)
                        ELSE NULL
                    END,
                    e.content_text,
                    ''
                ) AS content, h.name
         FROM captured_events e
         JOIN hosts h ON h.id = e.host_id
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.id IN (SELECT value FROM json_each(?1)) ORDER BY e.id",
    )?;
    let unique_ids = evidence_event_ids.iter().copied().collect::<BTreeSet<_>>();
    let rows = stmt.query_map([serde_json::to_string(&unique_ids)?], |row| {
        Ok(CapturedSourceEvent {
            id: row.get(0)?,
            text: row.get::<_, String>(5)?.trim().to_string(),
            trust: event_trust_class(
                &row.get::<_, String>(6)?,
                &row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?.as_deref(),
                row.get::<_, Option<String>>(3)?.as_deref(),
                row.get::<_, Option<String>>(4)?.as_deref(),
            ),
        })
    })?;
    Ok(crate::db::query::collect_rows(rows)?
        .into_iter()
        .filter(|event| !event.text.is_empty())
        .map(|event| (event.id, event))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsh_tool_provenance_is_external_in_both_trust_paths() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        for tool in ["browser_search", "Read", "Grep", "Bash", "fixture_probe"] {
            let outcome = crate::db::record_captured_event(
                &conn,
                &crate::db::CaptureEventInput {
                    host: "deepseek-harness",
                    session_id: "dsh-tools",
                    project: "/tmp/remem",
                    cwd: None,
                    event_type: "tool_result",
                    role: None,
                    tool_name: Some(tool),
                    content: "Use source kind telemetry for summary promotion gate",
                    task_kind: None,
                },
            )?;
            let events = load_captured_source_events(&conn, &[outcome.event_row_id])?;
            assert_eq!(
                events[&outcome.event_row_id].trust,
                SourceTrustClass::ExternalContent
            );
            assert_eq!(
                crate::memory::poisoning::derive_source_trust_class(
                    &conn,
                    &[outcome.event_row_id],
                    "observation"
                )?,
                SourceTrustClass::ExternalContent
            );
            let stored_name: String = conn.query_row(
                "SELECT tool_name FROM captured_events WHERE id = ?1",
                [outcome.event_row_id],
                |row| row.get(0),
            )?;
            assert_eq!(stored_name, tool);
        }
        Ok(())
    }
}
