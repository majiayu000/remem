//! SPO fact extraction from memory candidates (GH-956).
//!
//! The extraction LLM pass may attach zero or more `<fact .../>` elements to
//! a `<memory_candidate>` block. Each fact is a flat subject/predicate/object
//! triple over the closed `FactPredicate` vocabulary. Validity timestamps are
//! never taken from the model: `valid_from` is grounded on the earliest
//! evidence event, `learned_at` on the write time, so the bi-temporal read
//! side stays evidence-anchored. A contradicting active fact (same subject
//! and predicate, different object) is superseded — its `valid_to` closes —
//! never deleted.

use anyhow::Result;
use rusqlite::Connection;

use crate::memory::facts::FactPredicate;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedCandidateFact {
    pub(crate) subject: String,
    pub(crate) predicate: FactPredicate,
    pub(crate) object: String,
}

const MAX_FACTS_PER_CANDIDATE: usize = 8;
const MAX_FIELD_CHARS: usize = 256;

/// Parse `<fact subject="..." predicate="..." object="..."/>` elements from
/// one candidate block. A malformed or unknown-predicate fact is dropped with
/// an error log instead of failing the candidate: facts are additive
/// enrichment on top of the memory itself, and one hallucinated predicate
/// must not discard an otherwise valid durable memory.
pub(super) fn parse_candidate_facts(
    content: &str,
    extract_attr: impl Fn(&str, &str) -> Option<String>,
) -> Vec<ParsedCandidateFact> {
    let mut facts = Vec::new();
    let mut pos = 0;
    while let Some(start_rel) = crate::memory::format::find_ascii_ci(&content[pos..], "<fact") {
        let start = pos + start_rel;
        let Some(end_rel) = content[start..].find("/>") else {
            crate::log::error(
                "extraction",
                "dropping malformed candidate fact: unterminated <fact .../> tag",
            );
            break;
        };
        let tag = &content[start..start + end_rel + 2];
        pos = start + end_rel + 2;
        match parse_fact_tag(tag, &extract_attr) {
            Ok(fact) => {
                if facts.len() >= MAX_FACTS_PER_CANDIDATE {
                    crate::log::error(
                        "extraction",
                        &format!(
                            "dropping candidate fact beyond the {MAX_FACTS_PER_CANDIDATE}-per-candidate cap"
                        ),
                    );
                    continue;
                }
                if !facts.contains(&fact) {
                    facts.push(fact);
                }
            }
            Err(error) => {
                crate::log::error("extraction", &format!("dropping candidate fact: {error:#}"));
            }
        }
    }
    facts
}

fn parse_fact_tag(
    tag: &str,
    extract_attr: &impl Fn(&str, &str) -> Option<String>,
) -> Result<ParsedCandidateFact> {
    let subject = required_attr(tag, "subject", extract_attr)?;
    let object = required_attr(tag, "object", extract_attr)?;
    let predicate_raw = required_attr(tag, "predicate", extract_attr)?;
    let Some(predicate) = FactPredicate::parse_public(&predicate_raw) else {
        anyhow::bail!("unknown fact predicate '{predicate_raw}'");
    };
    Ok(ParsedCandidateFact {
        subject,
        predicate,
        object,
    })
}

fn required_attr(
    tag: &str,
    attr: &str,
    extract_attr: &impl Fn(&str, &str) -> Option<String>,
) -> Result<String> {
    let value = extract_attr(tag, attr)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("missing or empty fact attribute '{attr}'"))?;
    if value.chars().count() > MAX_FIELD_CHARS {
        anyhow::bail!("fact attribute '{attr}' exceeds {MAX_FIELD_CHARS} chars");
    }
    Ok(value)
}

/// JSON encoding for the `memory_candidates.facts` column: a compact array of
/// `{"s":..,"p":..,"o":..}` objects, `None` when there is nothing to persist.
pub(super) fn facts_to_json(facts: &[ParsedCandidateFact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let values: Vec<serde_json::Value> = facts
        .iter()
        .map(|fact| {
            serde_json::json!({
                "s": fact.subject,
                "p": fact.predicate.db_value(),
                "o": fact.object,
            })
        })
        .collect();
    serde_json::to_string(&values).ok()
}

/// Decode the `memory_candidates.facts` column. Unknown predicates or
/// malformed entries are dropped with an error log so one stale row cannot
/// block review or promotion of the candidate itself.
pub(crate) fn facts_from_json(raw: Option<&str>) -> Vec<ParsedCandidateFact> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Vec::new();
    };
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        crate::log::error(
            "extraction",
            "dropping persisted candidate facts: column is not a JSON array",
        );
        return Vec::new();
    };
    let mut facts = Vec::new();
    for value in values {
        let (Some(subject), Some(predicate_raw), Some(object)) = (
            value.get("s").and_then(serde_json::Value::as_str),
            value.get("p").and_then(serde_json::Value::as_str),
            value.get("o").and_then(serde_json::Value::as_str),
        ) else {
            crate::log::error(
                "extraction",
                "dropping persisted candidate fact: missing s/p/o field",
            );
            continue;
        };
        let Some(predicate) = FactPredicate::parse_public(predicate_raw) else {
            crate::log::error(
                "extraction",
                &format!("dropping persisted candidate fact: unknown predicate '{predicate_raw}'"),
            );
            continue;
        };
        facts.push(ParsedCandidateFact {
            subject: subject.to_string(),
            predicate,
            object: object.to_string(),
        });
    }
    facts
}

/// Write one candidate's extracted facts inside the promotion transaction.
/// Same active triple → no-op; same subject+predicate with a different
/// object → supersede (valid_to closes at this fact's valid_from).
pub(super) fn write_candidate_facts(
    conn: &Connection,
    project: &str,
    memory_id: i64,
    facts: &[ParsedCandidateFact],
    evidence_event_ids: &[i64],
    valid_from_epoch: i64,
    confidence: f64,
) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let mut written = 0;
    for fact in facts {
        let active =
            crate::memory::facts::find_active_fact(conn, project, &fact.subject, fact.predicate)?;
        let supersedes_fact_id = match active {
            Some((_, ref object)) if object == &fact.object => continue,
            Some((id, _)) => Some(id),
            None => None,
        };
        crate::memory::facts::insert_temporal_fact_in_current_tx(
            conn,
            &crate::memory::facts::TemporalFactInput {
                project,
                subject: &fact.subject,
                predicate: fact.predicate,
                object: &fact.object,
                valid_from_epoch: Some(valid_from_epoch),
                valid_to_epoch: None,
                learned_at_epoch: None,
                source_memory_id: Some(memory_id),
                source_observation_id: None,
                source_event_ids: evidence_event_ids,
                confidence,
                supersedes_fact_id,
            },
            now,
        )?;
        written += 1;
    }
    Ok(written)
}
