use crate::memory::poisoning::SourceTrustClass;
use crate::memory::MemoryType;
use crate::runtime_config::SummaryGateMode;

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
    source_trust: SourceTrustClass,
) -> bool {
    candidate.scope == "project"
        && candidate.risk_class == "low"
        && candidate.confidence >= AUTO_PROMOTE_MIN_CONFIDENCE
        && source_trust.allows_auto_promote()
        && route.is_repo_owned()
        && route.routing_confidence >= AUTO_PROMOTE_MIN_CONFIDENCE
        && has_evidence_ids(evidence_json)
        && MemoryType::parse(&candidate.memory_type).is_some_and(MemoryType::auto_promote)
        && !contains_auto_promote_unsafe_marker(&candidate.text)
        && is_supported_by_source_observation(candidate, batch)
}

pub(super) enum CandidatePromotionDecision {
    Promote,
    PendingReview {
        block_reason: &'static str,
        summary_shadow_promoted: bool,
    },
}

pub(super) fn candidate_promotion_decision(
    candidate: &ParsedMemoryCandidate,
    auto_promote_batch: Option<&ObservationBatch>,
    route: &CandidateRoute,
    evidence_json: &str,
    source_kind: &str,
    source_trust: SourceTrustClass,
    summary_gate_mode: Option<SummaryGateMode>,
    source_texts: &[&str],
) -> CandidatePromotionDecision {
    if source_kind == super::SOURCE_KIND_SUMMARY {
        let Some(mode) = summary_gate_mode else {
            return CandidatePromotionDecision::PendingReview {
                block_reason: "summary_gate_mode_missing",
                summary_shadow_promoted: false,
            };
        };
        if mode == SummaryGateMode::Off {
            return CandidatePromotionDecision::PendingReview {
                block_reason: "summary_gate_off",
                summary_shadow_promoted: false,
            };
        }
        return match summary_auto_promote_verdict(
            candidate,
            route,
            evidence_json,
            source_texts,
            source_trust,
        ) {
            SummaryAutoPromoteVerdict::WouldPromote if mode == SummaryGateMode::Enforce => {
                CandidatePromotionDecision::Promote
            }
            SummaryAutoPromoteVerdict::WouldPromote => CandidatePromotionDecision::PendingReview {
                block_reason: "summary_gate_shadow",
                summary_shadow_promoted: true,
            },
            SummaryAutoPromoteVerdict::Blocked(block_reason) => {
                CandidatePromotionDecision::PendingReview {
                    block_reason,
                    summary_shadow_promoted: false,
                }
            }
        };
    }

    if auto_promote_batch.is_some_and(|batch| {
        should_auto_promote(candidate, batch, route, evidence_json, source_trust)
    }) {
        CandidatePromotionDecision::Promote
    } else {
        CandidatePromotionDecision::PendingReview {
            block_reason: auto_promote_block_reason(
                candidate,
                auto_promote_batch,
                route,
                evidence_json,
                source_trust,
            ),
            summary_shadow_promoted: false,
        }
    }
}

/// Explain why a candidate did not auto-promote, mirroring the checks in
/// `should_auto_promote`. Used for observability when a candidate is routed to
/// pending_review (U-29: a downgrade with user-visible effect must be logged).
pub(super) fn auto_promote_block_reason(
    candidate: &ParsedMemoryCandidate,
    batch: Option<&ObservationBatch>,
    route: &CandidateRoute,
    evidence_json: &str,
    source_trust: SourceTrustClass,
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
    if !source_trust.allows_auto_promote() {
        return "source_trust_below_floor";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SummaryAutoPromoteVerdict {
    WouldPromote,
    Blocked(&'static str),
}

pub(super) fn summary_auto_promote_verdict(
    candidate: &ParsedMemoryCandidate,
    route: &CandidateRoute,
    evidence_json: &str,
    source_texts: &[&str],
    source_trust: SourceTrustClass,
) -> SummaryAutoPromoteVerdict {
    if candidate.scope != "project" {
        return SummaryAutoPromoteVerdict::Blocked("scope_not_project");
    }
    if !summary_type_allowlisted(&candidate.memory_type) {
        return SummaryAutoPromoteVerdict::Blocked("summary_type_not_allowlisted");
    }
    if candidate.confidence < SUMMARY_AUTO_PROMOTE_MIN_CONFIDENCE {
        return SummaryAutoPromoteVerdict::Blocked("summary_confidence_below_floor");
    }
    if !source_trust.allows_auto_promote() {
        return SummaryAutoPromoteVerdict::Blocked("source_trust_below_floor");
    }
    if !route.is_repo_owned() {
        return SummaryAutoPromoteVerdict::Blocked("route_not_repo_owned");
    }
    if route.routing_confidence < AUTO_PROMOTE_MIN_CONFIDENCE {
        return SummaryAutoPromoteVerdict::Blocked("routing_confidence_below_threshold");
    }
    if !has_evidence_ids(evidence_json) {
        return SummaryAutoPromoteVerdict::Blocked("missing_evidence_ids");
    }
    if contains_auto_promote_unsafe_marker(&candidate.text) {
        return SummaryAutoPromoteVerdict::Blocked("contains_unsafe_marker");
    }
    if !summary_risk_allowed(&candidate.risk_class) {
        return SummaryAutoPromoteVerdict::Blocked("summary_risk_above_medium");
    }
    match is_supported_by_summary_source(candidate, source_texts) {
        SummarySupport::Supported => SummaryAutoPromoteVerdict::WouldPromote,
        SummarySupport::Unavailable => {
            SummaryAutoPromoteVerdict::Blocked("summary_source_support_unavailable")
        }
        SummarySupport::Failed => {
            SummaryAutoPromoteVerdict::Blocked("summary_source_support_failed")
        }
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
