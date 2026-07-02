use crate::memory::MemoryType;

use super::route::CandidateRoute;
use super::support::has_conservative_source_support;
use super::{ObservationBatch, ParsedMemoryCandidate};

const AUTO_PROMOTE_MIN_CONFIDENCE: f64 = 0.80;
const AUTO_PROMOTE_MIN_OBSERVATION_CONFIDENCE: f64 = 0.75;
const SUMMARY_AUTO_PROMOTE_MIN_CONFIDENCE: f64 = 0.70;
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

pub(super) fn should_auto_promote(
    candidate: &ParsedMemoryCandidate,
    batch: &ObservationBatch,
    route: &CandidateRoute,
    evidence_json: &str,
) -> bool {
    candidate.scope == "project"
        && candidate.risk_class == "low"
        && candidate.confidence >= AUTO_PROMOTE_MIN_CONFIDENCE
        && route.is_repo_owned()
        && route.routing_confidence >= AUTO_PROMOTE_MIN_CONFIDENCE
        && has_evidence_ids(evidence_json)
        && MemoryType::parse(&candidate.memory_type).is_some_and(MemoryType::auto_promote)
        && !contains_auto_promote_unsafe_marker(&candidate.text)
        && is_supported_by_source_observation(candidate, batch)
}

/// Explain why a candidate did not auto-promote, mirroring the checks in
/// `should_auto_promote`. Used for observability when a candidate is routed to
/// pending_review (U-29: a downgrade with user-visible effect must be logged).
pub(super) fn auto_promote_block_reason(
    candidate: &ParsedMemoryCandidate,
    batch: Option<&ObservationBatch>,
    route: &CandidateRoute,
    evidence_json: &str,
) -> &'static str {
    if candidate.scope != "project" {
        return "scope_not_project";
    }
    if candidate.risk_class != "low" {
        return "risk_class_not_low";
    }
    if candidate.confidence < AUTO_PROMOTE_MIN_CONFIDENCE {
        return "confidence_below_threshold";
    }
    if !route.is_repo_owned() {
        return "route_not_repo_owned";
    }
    if route.routing_confidence < AUTO_PROMOTE_MIN_CONFIDENCE {
        return "routing_confidence_below_threshold";
    }
    if !has_evidence_ids(evidence_json) {
        return "missing_evidence_ids";
    }
    if !MemoryType::parse(&candidate.memory_type).is_some_and(MemoryType::auto_promote) {
        return "memory_type_not_auto_promotable";
    }
    if contains_auto_promote_unsafe_marker(&candidate.text) {
        return "contains_unsafe_marker";
    }
    let Some(batch) = batch else {
        return "missing_source_observation_batch";
    };
    if !is_supported_by_source_observation(candidate, batch) {
        return "no_supporting_source_observation";
    }
    "unknown"
}

pub(super) fn summary_auto_promote_block_reason(
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
    evidence_json: &str,
    source_texts: &[&str],
) -> &'static str {
    if candidate.scope != "project" {
        return "scope_not_project";
    }
    if !summary_type_allowlisted(&candidate.memory_type) {
        return "summary_type_not_allowlisted";
    }
    if candidate.confidence < SUMMARY_AUTO_PROMOTE_MIN_CONFIDENCE {
        return "summary_confidence_below_floor";
    }
    if !route.is_repo_owned() {
        return "route_not_repo_owned";
    }
    if route.routing_confidence < AUTO_PROMOTE_MIN_CONFIDENCE {
        return "routing_confidence_below_threshold";
    }
    if !has_evidence_ids(evidence_json) {
        return "missing_evidence_ids";
    }
    if contains_auto_promote_unsafe_marker(&candidate.text) {
        return "contains_unsafe_marker";
    }
    if !summary_risk_allowed(&candidate.risk_class) {
        return "summary_risk_above_medium";
    }
    match is_supported_by_summary_source(candidate, source_texts) {
        SummarySupport::Supported => "summary_gate_shadow",
        SummarySupport::Unavailable => "summary_source_support_unavailable",
        SummarySupport::Failed => "summary_source_support_failed",
    }
}

fn has_evidence_ids(evidence_json: &str) -> bool {
    serde_json::from_str::<Vec<i64>>(evidence_json).is_ok_and(|ids| !ids.is_empty())
}

fn is_supported_by_source_observation(
    candidate: &ParsedMemoryCandidate,
    batch: &ObservationBatch,
) -> bool {
    let candidate_text = normalize_evidence_text(&candidate.text);
    if candidate_text.chars().count() < 24 {
        return false;
    }
    let Some(candidate_type) = MemoryType::parse(&candidate.memory_type) else {
        return false;
    };
    batch.observations.iter().any(|observation| {
        if observation.confidence < AUTO_PROMOTE_MIN_OBSERVATION_CONFIDENCE
            || !candidate_type.supports_observation_type(&observation.observation_type)
        {
            return false;
        }
        let observation_text = normalize_evidence_text(&observation.text);
        has_conservative_source_support(&candidate_text, &observation_text)
    })
}

fn summary_type_allowlisted(memory_type: &str) -> bool {
    matches!(
        MemoryType::parse(memory_type),
        Some(MemoryType::Decision | MemoryType::Discovery)
    )
}

fn summary_risk_allowed(risk_class: &str) -> bool {
    matches!(risk_class, "low" | "medium")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummarySupport {
    Supported,
    Unavailable,
    Failed,
}

fn is_supported_by_summary_source(
    candidate: &ParsedMemoryCandidate,
    source_texts: &[&str],
) -> SummarySupport {
    let source_texts = source_texts
        .iter()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>();
    if source_texts.is_empty() {
        return SummarySupport::Unavailable;
    }

    let candidate_text = normalize_evidence_text(&candidate.text);
    if candidate_text.chars().count() < 24 {
        return SummarySupport::Failed;
    }
    if source_texts.iter().any(|source_text| {
        let source_text = normalize_evidence_text(source_text);
        has_conservative_source_support(&candidate_text, &source_text)
    }) {
        SummarySupport::Supported
    } else {
        SummarySupport::Failed
    }
}

pub(super) fn contains_auto_promote_unsafe_marker(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    AUTO_PROMOTE_UNSAFE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

pub(crate) fn contains_unsafe_memory_marker(text: &str) -> bool {
    contains_auto_promote_unsafe_marker(text)
}

fn normalize_evidence_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}
