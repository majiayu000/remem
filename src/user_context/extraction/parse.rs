use anyhow::{bail, ensure, Context, Result};
use serde::Deserialize;

use crate::user_context::candidates::UserContextCandidateRisk;
use crate::user_context::claims::{UserContextClaimType, UserContextSensitivity};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParsedUserContextCandidate {
    pub(crate) claim_type: UserContextClaimType,
    pub(crate) claim_key: String,
    pub(crate) claim_text: String,
    pub(crate) confidence: f64,
    pub(crate) sensitivity: UserContextSensitivity,
    pub(crate) risk_class: UserContextCandidateRisk,
    pub(crate) source_kind: String,
    pub(crate) source_event_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum UserContextCandidateResponse {
    NoCandidates,
    Candidates(Vec<ParsedUserContextCandidate>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseEnvelope {
    candidates: Option<Vec<JsonCandidate>>,
    no_candidates: Option<NoCandidates>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoCandidates {
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonCandidate {
    claim_type: String,
    claim_key: String,
    claim_text: String,
    confidence: f64,
    sensitivity: String,
    risk_class: String,
    source_kind: String,
    source_event_ids: Vec<i64>,
}

pub(crate) fn parse_user_context_candidate_response(
    output: &str,
) -> Result<UserContextCandidateResponse> {
    let envelope: ResponseEnvelope = serde_json::from_str(output.trim())
        .context("malformed user_context_candidate output: expected strict JSON object")?;
    match (envelope.candidates, envelope.no_candidates) {
        (Some(_), Some(_)) => bail!(
            "malformed user_context_candidate output: candidates and no_candidates are mutually exclusive"
        ),
        (None, None) => bail!(
            "malformed user_context_candidate output: missing candidates or no_candidates"
        ),
        (None, Some(no_candidates)) => {
            ensure!(
                !no_candidates.reason.trim().is_empty(),
                "malformed user_context_candidate output: no_candidates.reason is required"
            );
            Ok(UserContextCandidateResponse::NoCandidates)
        }
        (Some(candidates), None) => {
            ensure!(
                !candidates.is_empty(),
                "malformed user_context_candidate output: candidates must not be empty"
            );
            candidates
                .into_iter()
                .enumerate()
                .map(|(index, candidate)| validate_candidate(index, candidate))
                .collect::<Result<Vec<_>>>()
                .map(UserContextCandidateResponse::Candidates)
        }
    }
}

fn validate_candidate(
    index: usize,
    candidate: JsonCandidate,
) -> Result<ParsedUserContextCandidate> {
    let claim_type = parse_claim_type(&candidate.claim_type, index)?;
    let claim_key = normalize_claim_key(&candidate.claim_key, index)?;
    let claim_text = normalize_required("claim_text", index, &candidate.claim_text)?;
    let confidence = validate_confidence(candidate.confidence, index)?;
    let sensitivity = parse_sensitivity(&candidate.sensitivity, index)?;
    let risk_class = parse_risk_class(&candidate.risk_class, index)?;
    let source_kind = parse_source_kind(&candidate.source_kind, index)?;
    let source_event_ids = validate_source_event_ids(candidate.source_event_ids, index)?;
    Ok(ParsedUserContextCandidate {
        claim_type,
        claim_key,
        claim_text,
        confidence,
        sensitivity,
        risk_class,
        source_kind,
        source_event_ids,
    })
}

fn parse_claim_type(raw: &str, index: usize) -> Result<UserContextClaimType> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "identity" => Ok(UserContextClaimType::Identity),
        "role" => Ok(UserContextClaimType::Role),
        "preference" => Ok(UserContextClaimType::Preference),
        "skill" => Ok(UserContextClaimType::Skill),
        "goal" => Ok(UserContextClaimType::Goal),
        "project" => Ok(UserContextClaimType::Project),
        "relationship" => Ok(UserContextClaimType::Relationship),
        "constraint" => Ok(UserContextClaimType::Constraint),
        "activity" => Ok(UserContextClaimType::Activity),
        other => bail!(
            "malformed user_context_candidate output: candidate {index} has unsupported claim_type `{other}`"
        ),
    }
}

fn parse_sensitivity(raw: &str, index: usize) -> Result<UserContextSensitivity> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "normal" => Ok(UserContextSensitivity::Normal),
        "personal" => Ok(UserContextSensitivity::Personal),
        "sensitive" => Ok(UserContextSensitivity::Sensitive),
        "restricted" => Ok(UserContextSensitivity::Restricted),
        other => bail!(
            "malformed user_context_candidate output: candidate {index} has unsupported sensitivity `{other}`"
        ),
    }
}

fn parse_risk_class(raw: &str, index: usize) -> Result<UserContextCandidateRisk> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "low" => Ok(UserContextCandidateRisk::Low),
        "medium" => Ok(UserContextCandidateRisk::Medium),
        "high" => Ok(UserContextCandidateRisk::High),
        other => bail!(
            "malformed user_context_candidate output: candidate {index} has unsupported risk_class `{other}`"
        ),
    }
}

fn parse_source_kind(raw: &str, index: usize) -> Result<String> {
    let value = raw.trim().to_ascii_lowercase();
    match value.as_str() {
        "explicit_user_statement"
        | "inferred_from_behavior"
        | "session_summary"
        | "third_party_statement"
        | "speculative_inference" => Ok(value),
        other => bail!(
            "malformed user_context_candidate output: candidate {index} has unsupported source_kind `{other}`"
        ),
    }
}

fn normalize_claim_key(raw: &str, index: usize) -> Result<String> {
    let value = raw.trim().to_ascii_lowercase();
    ensure!(
        !value.is_empty(),
        "malformed user_context_candidate output: candidate {index} claim_key is required"
    );
    ensure!(
        !value.chars().any(char::is_whitespace),
        "malformed user_context_candidate output: candidate {index} claim_key must not contain whitespace"
    );
    Ok(value)
}

fn normalize_required(field: &str, index: usize, raw: &str) -> Result<String> {
    let value = raw.trim();
    ensure!(
        !value.is_empty(),
        "malformed user_context_candidate output: candidate {index} {field} is required"
    );
    Ok(value.to_string())
}

fn validate_confidence(value: f64, index: usize) -> Result<f64> {
    ensure!(
        value.is_finite() && (0.0..=1.0).contains(&value),
        "malformed user_context_candidate output: candidate {index} confidence must be between 0.0 and 1.0"
    );
    Ok(value)
}

fn validate_source_event_ids(values: Vec<i64>, index: usize) -> Result<Vec<i64>> {
    ensure!(
        !values.is_empty(),
        "malformed user_context_candidate output: candidate {index} source_event_ids must not be empty"
    );
    for id in &values {
        ensure!(
            *id > 0,
            "malformed user_context_candidate output: candidate {index} source_event_ids must be positive"
        );
    }
    Ok(values)
}
