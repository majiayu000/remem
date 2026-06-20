use std::future::Future;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::memory_candidate::support::has_conservative_source_support;

use super::candidates::{self, CandidateCreateRequest};
use super::claims::{DEFAULT_OWNER_KEY, DEFAULT_OWNER_SCOPE};

mod parse;
mod prompt;
mod source;
#[cfg(test)]
mod tests;

use parse::UserContextCandidateResponse;
pub(crate) use parse::{parse_user_context_candidate_response, ParsedUserContextCandidate};
use source::{load_source_batch, CandidateSourceBatch};

const USER_CONTEXT_CANDIDATE_SYSTEM: &str = "\
Extract review-gated user-context candidates from captured development-session events.
Return only one strict JSON object, with no markdown, prose, or XML.
Use {\"candidates\":[...]} when stable user-context claims are evidenced, or
{\"no_candidates\":{\"reason\":\"...\"}} when they are not.
Every candidate must include exactly: claim_type, claim_key, claim_text, confidence,
sensitivity, risk_class, source_kind, source_event_ids.
Allowed claim_type values: identity, role, preference, skill, goal, project,
relationship, constraint, activity.
Allowed sensitivity values: normal, personal, sensitive, restricted.
Allowed risk_class values: low, medium, high.
Allowed source_kind values: explicit_user_statement, inferred_from_behavior,
session_summary, third_party_statement, speculative_inference.
Use risk_class=low only for normal-sensitivity preference or constraint claims
that are explicitly stated by the user and cite user-authored source_event_ids.
Sensitive, restricted, inferred, speculative, relationship, health, political,
religious, identity, role, location, and assistant-authored claims must stay review-gated.
Use only provided events and summaries; do not invent private facts or preferences.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UserContextCandidateExtractResult {
    EmptyRange,
    NoCandidates {
        to_event_id: i64,
    },
    Written {
        candidates: usize,
        promoted: usize,
        pending_review: usize,
        to_event_id: i64,
    },
}

pub(crate) async fn process(
    task: &db::ExtractionTask,
) -> Result<UserContextCandidateExtractResult> {
    let mut conn = db::open_db()?;
    let project = task.project.clone();
    let ai_profile = task.ai_profile.clone();
    process_with_generator(&mut conn, task, move |prompt| {
        let project = project.clone();
        let ai_profile = ai_profile.clone();
        async move {
            let profile = ai_profile.as_deref();
            crate::ai::call_ai(
                USER_CONTEXT_CANDIDATE_SYSTEM,
                &prompt,
                crate::ai::UsageContext {
                    project: Some(project.as_str()),
                    session_id: task.session_id.as_deref(),
                    operation: "user_context_candidate",
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
) -> Result<UserContextCandidateExtractResult>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let Some(batch) = load_source_batch(conn, task)? else {
        return Ok(UserContextCandidateExtractResult::EmptyRange);
    };
    let prompt = prompt::build_candidate_prompt(task, &batch)?;
    let response = generate(prompt).await?;
    let parsed = match parse_user_context_candidate_response(&response)? {
        UserContextCandidateResponse::NoCandidates => {
            return Ok(UserContextCandidateExtractResult::NoCandidates {
                to_event_id: batch.to_event_id,
            });
        }
        UserContextCandidateResponse::Candidates(candidates) => candidates,
    };
    validate_candidate_sources(&batch, &parsed)?;
    let summary = persist_candidates(conn, task, &batch, &parsed)?;
    crate::log::info(
        "user-context-candidate",
        &format!(
            "session={} range={}..{} candidates={} promoted={} pending_review={}",
            task.session_id.as_deref().unwrap_or("<unknown>"),
            batch.from_event_id,
            batch.to_event_id,
            summary.candidates,
            summary.promoted,
            summary.pending_review
        ),
    );
    Ok(UserContextCandidateExtractResult::Written {
        candidates: summary.candidates,
        promoted: summary.promoted,
        pending_review: summary.pending_review,
        to_event_id: batch.to_event_id,
    })
}

#[derive(Default)]
struct PersistSummary {
    candidates: usize,
    promoted: usize,
    pending_review: usize,
}

fn persist_candidates(
    conn: &Connection,
    task: &db::ExtractionTask,
    batch: &CandidateSourceBatch,
    parsed: &[ParsedUserContextCandidate],
) -> Result<PersistSummary> {
    let mut summary = PersistSummary::default();
    for candidate in parsed {
        let source_refs_json = source::source_refs_json(batch, candidate)?;
        if candidate_exists(conn, candidate, &source_refs_json)? {
            continue;
        }
        let auto_promote = should_auto_promote(candidate, batch);
        let result = candidates::create_candidate(
            conn,
            &CandidateCreateRequest {
                text: &candidate.claim_text,
                owner_scope: None,
                owner_key: None,
                source_project: Some(&task.project),
                host: Some(&task.host),
                session_id: task.session_id.as_deref(),
                claim_type: candidate.claim_type,
                claim_key: Some(&candidate.claim_key),
                confidence: candidate.confidence,
                sensitivity: candidate.sensitivity,
                risk_class: candidate.risk_class,
                source_kind: &candidate.source_kind,
                source_refs_json: &source_refs_json,
                source_preview: source::source_preview(batch, candidate).as_deref(),
                auto_promote,
                auto_promote_block_reason: (!auto_promote)
                    .then(|| auto_promote_block_reason(candidate, batch)),
            },
        )?;
        summary.candidates += 1;
        if result.candidate.review_status == "auto_promoted" {
            summary.promoted += 1;
        } else {
            summary.pending_review += 1;
        }
    }
    Ok(summary)
}

fn should_auto_promote(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> bool {
    matches!(
        candidate.claim_type,
        super::claims::UserContextClaimType::Preference
            | super::claims::UserContextClaimType::Constraint
    ) && candidate.risk_class == super::candidates::UserContextCandidateRisk::Low
        && candidate.sensitivity == super::claims::UserContextSensitivity::Normal
        && candidate.confidence >= 0.9
        && candidate.source_kind == "explicit_user_statement"
        && candidate
            .source_event_ids
            .iter()
            .all(|id| batch.event_is_user_authored(*id))
        && is_supported_by_user_source_event(candidate, batch)
}

fn auto_promote_block_reason(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> &'static str {
    if !matches!(
        candidate.claim_type,
        super::claims::UserContextClaimType::Preference
            | super::claims::UserContextClaimType::Constraint
    ) {
        return "claim_type_requires_review";
    }
    if candidate.risk_class != super::candidates::UserContextCandidateRisk::Low {
        return "risk_requires_review";
    }
    if candidate.sensitivity != super::claims::UserContextSensitivity::Normal {
        return "sensitivity_requires_review";
    }
    if candidate.confidence < 0.9 {
        return "low_confidence";
    }
    if candidate.source_kind != "explicit_user_statement" {
        return "source_requires_review";
    }
    if !candidate
        .source_event_ids
        .iter()
        .all(|id| batch.event_is_user_authored(*id))
    {
        return "source_not_user_authored";
    }
    if !is_supported_by_user_source_event(candidate, batch) {
        return "no_supporting_source_event";
    }
    "requires_review"
}

fn is_supported_by_user_source_event(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> bool {
    let candidate_text = normalize_support_text(&candidate.claim_text);
    let short_variants = short_user_context_support_variants(&candidate.claim_text);
    batch
        .events_for_candidate(candidate)
        .into_iter()
        .any(|event| {
            if !batch.event_is_user_authored(event.id) {
                return false;
            }
            let event_text = normalize_support_text(&event.content);
            has_conservative_source_support(&candidate_text, &event_text)
                || short_variants
                    .iter()
                    .any(|variant| has_short_exact_source_support(variant, &event_text))
        })
}

fn normalize_support_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn short_user_context_support_variants(text: &str) -> Vec<Vec<String>> {
    let tokens = short_support_tokens(text);
    let mut variants = vec![tokens.clone()];
    for prefix in [
        &["the", "user"][..],
        &["user"][..],
        &["i"][..],
        &["we"][..],
        &["my"][..],
        &["our"][..],
    ] {
        if tokens.len() > prefix.len()
            && prefix
                .iter()
                .enumerate()
                .all(|(index, expected)| tokens.get(index).is_some_and(|token| token == expected))
        {
            variants.push(tokens[prefix.len()..].to_vec());
        }
    }
    variants.sort();
    variants.dedup();
    variants
}

fn has_short_exact_source_support(candidate_tokens: &[String], source_text: &str) -> bool {
    if !(3..=8).contains(&candidate_tokens.len()) || contains_short_support_risk(candidate_tokens) {
        return false;
    }
    let source_tokens = short_support_tokens(source_text);
    if source_tokens.len() < candidate_tokens.len() || contains_short_support_risk(&source_tokens) {
        return false;
    }
    source_tokens
        .windows(candidate_tokens.len())
        .any(|window| window == candidate_tokens)
}

fn short_support_tokens(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| normalize_short_support_token(&token.to_ascii_lowercase()))
        .collect()
}

fn normalize_short_support_token(token: &str) -> String {
    if let Some(stem) = token.strip_suffix("ies") {
        return format!("{stem}y");
    }
    if token.len() > 4 && token.ends_with('s') && !token.ends_with("ss") && !token.ends_with("us") {
        return token[..token.len() - 1].to_string();
    }
    token.to_string()
}

fn contains_short_support_risk(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "cannot"
                | "cant"
                | "could"
                | "deny"
                | "disable"
                | "doesn"
                | "don"
                | "ignore"
                | "may"
                | "might"
                | "must"
                | "never"
                | "no"
                | "not"
                | "should"
                | "shouldn"
                | "skip"
                | "unless"
                | "without"
                | "would"
                | "wouldn"
        )
    })
}

fn validate_candidate_sources(
    batch: &CandidateSourceBatch,
    candidates: &[ParsedUserContextCandidate],
) -> Result<()> {
    for (index, candidate) in candidates.iter().enumerate() {
        for id in &candidate.source_event_ids {
            if !batch.has_event(*id) {
                bail!(
                    "malformed user_context_candidate output: candidate {index} cites event id {id} outside loaded source range"
                );
            }
        }
    }
    Ok(())
}

fn candidate_exists(
    conn: &Connection,
    candidate: &ParsedUserContextCandidate,
    source_refs_json: &str,
) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM user_context_candidates
             WHERE owner_scope = ?1
               AND owner_key = ?2
               AND claim_type = ?3
               AND claim_key = ?4
               AND claim_text = ?5
               AND source_refs_json = ?6
             LIMIT 1",
            params![
                DEFAULT_OWNER_SCOPE,
                DEFAULT_OWNER_KEY,
                candidate.claim_type.db_value(),
                candidate.claim_key,
                candidate.claim_text,
                source_refs_json,
            ],
            |row| row.get(0),
        )
        .optional()?;
    Ok(existing.is_some())
}
