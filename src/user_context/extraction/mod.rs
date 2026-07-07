use std::future::Future;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::db;
use crate::memory_candidate::support::has_conservative_source_support;
use crate::runtime_config::AutoPromotePolicy;

use super::candidates::{self, CandidateCreateRequest};
use super::claims::{DEFAULT_OWNER_KEY, DEFAULT_OWNER_SCOPE};

mod parse;
#[cfg(test)]
mod policy_tests;
mod promotion_gate;
mod prompt;
#[cfg(test)]
mod review_feedback_tests;
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
    let policy = crate::runtime_config::user_context_auto_promote_config()?.effective_policy();
    process_with_generator_with_policy(&mut conn, task, &policy, move |prompt| {
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

#[cfg(test)]
async fn process_with_generator<F, Fut>(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    generate: F,
) -> Result<UserContextCandidateExtractResult>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let policy = AutoPromotePolicy::relaxed_default();
    process_with_generator_with_policy(conn, task, &policy, generate).await
}

#[cfg(test)]
async fn process_with_generator_strict<F, Fut>(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    generate: F,
) -> Result<UserContextCandidateExtractResult>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let policy = AutoPromotePolicy::strict();
    process_with_generator_with_policy(conn, task, &policy, generate).await
}

async fn process_with_generator_with_policy<F, Fut>(
    conn: &mut Connection,
    task: &db::ExtractionTask,
    policy: &AutoPromotePolicy,
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
    let summary = persist_candidates(conn, task, &batch, &parsed, policy)?;
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
    blocked: usize,
}

fn persist_candidates(
    conn: &Connection,
    task: &db::ExtractionTask,
    batch: &CandidateSourceBatch,
    parsed: &[ParsedUserContextCandidate],
    policy: &AutoPromotePolicy,
) -> Result<PersistSummary> {
    let mut summary = PersistSummary::default();
    for candidate in parsed {
        let source_evidence = source::source_evidence_text(batch, candidate);
        let source_preview = source_evidence
            .as_deref()
            .map(|preview| crate::db::truncate_str(preview, 500).to_string());
        if let Some(reason) =
            non_retention_block_reason(candidate, batch, source_evidence.as_deref())
        {
            summary.blocked += 1;
            crate::log::info(
                "user-context-candidate",
                &format!("blocked non-retention candidate reason={reason}"),
            );
            continue;
        }
        let source_refs_json = source::source_refs_json(batch, candidate)?;
        if candidate_exists(conn, candidate, &source_refs_json)? {
            continue;
        }
        let auto_promote = promotion_gate::is_auto_promote_allowed(candidate, batch, policy);
        let result = candidates::create_candidate_with_policy(
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
                source_preview: source_preview.as_deref(),
                auto_promote,
                auto_promote_block_reason: (!auto_promote)
                    .then(|| promotion_gate::blocked_reason(candidate, batch, policy)),
            },
            policy,
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

fn non_retention_block_reason(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
    source_preview: Option<&str>,
) -> Option<&'static str> {
    crate::user_context::non_retention::block_reason(
        &candidate.claim_text,
        source_preview,
        &candidate.source_kind,
    )
    .or_else(|| {
        (requires_third_party_framing(candidate)
            && !is_supported_third_party_candidate(candidate, batch))
        .then_some("unframed_third_party_detail")
    })
    .or_else(|| {
        (!requires_third_party_framing(candidate)
            && !is_supported_for_candidate_queue(candidate, batch))
        .then_some("no_supporting_user_source_event")
    })
}

fn requires_third_party_framing(candidate: &ParsedUserContextCandidate) -> bool {
    candidate.source_kind == "third_party_statement"
        || candidate.claim_type == super::claims::UserContextClaimType::Relationship
        || claim_text_describes_third_party_fact(&candidate.claim_text)
}

fn claim_text_describes_third_party_fact(text: &str) -> bool {
    let tokens = short_support_tokens(text);
    claim_has_likely_third_party_subject(text) || claim_has_user_owned_third_party_role(&tokens)
}

fn claim_has_likely_third_party_subject(text: &str) -> bool {
    let mut words = text
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '\'' || ch == '-'))
        .filter(|word| !word.is_empty());
    let first = match words.next() {
        Some("The" | "the") => words.next(),
        other => other,
    };
    first.is_some_and(|word| {
        is_likely_third_party_name(word)
            || (is_likely_lowercase_third_party_subject(word)
                && claim_has_third_party_predicate(text)
                && !claim_is_user_preferred_subject_statement(text))
    })
}

fn is_likely_third_party_name(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() || !chars.any(|ch| ch.is_ascii_lowercase()) {
        return false;
    }
    !NON_THIRD_PARTY_NAME_SUBJECTS.contains(&format!("|{}|", word.to_ascii_lowercase()))
}

fn is_likely_lowercase_third_party_subject(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.any(|ch| ch.is_ascii_lowercase())
        && !NON_THIRD_PARTY_NAME_SUBJECTS.contains(&format!("|{}|", word.to_ascii_lowercase()))
}

fn claim_has_third_party_predicate(text: &str) -> bool {
    short_support_tokens(text).iter().any(|token| {
        matches!(
            token.as_str(),
            "live" | "own" | "prefer" | "use" | "want" | "work"
        )
    })
}

fn claim_is_user_preferred_subject_statement(text: &str) -> bool {
    let tokens = short_support_tokens(text);
    let has_user_subject = tokens
        .iter()
        .any(|token| matches!(token.as_str(), "user" | "my"));
    let has_preference = tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "favorite" | "favourite" | "prefer" | "preferred"
        )
    });
    has_user_subject && has_preference
}

const NON_THIRD_PARTY_NAME_SUBJECTS: &str = "|api|browser|cargo|claude|code|codebase|codex|github|gitlab|javascript|json|linux|macos|mcp|node|npm|openai|page|project|python|readme|repo|repository|review|reviews|rust|setting|settings|sqlite|sql|task|test|testing|the|toml|typescript|user|web|windows|workspace|yaml|";

fn claim_has_user_owned_third_party_role(tokens: &[String]) -> bool {
    has_user_owned_subject(tokens) && has_third_party_role_token(tokens)
}

fn has_user_owned_subject(tokens: &[String]) -> bool {
    tokens
        .first()
        .is_some_and(|token| matches!(token.as_str(), "my" | "our"))
        || tokens.windows(2).any(|window| window == ["user", "s"])
        || tokens
            .windows(3)
            .any(|window| window == ["the", "user", "s"])
}

fn has_third_party_role_token(tokens: &[String]) -> bool {
    tokens
        .iter()
        .any(|token| THIRD_PARTY_ROLE_TOKENS.contains(&format!("|{token}|")))
}

const THIRD_PARTY_ROLE_TOKENS: &str = "|approver|client|colleague|collaborator|coworker|family|father|friend|husband|manager|mentor|mother|partner|reviewer|sibling|spouse|stakeholder|teammate|vendor|wife|";

fn is_supported_third_party_candidate(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> bool {
    batch
        .events_for_candidate(candidate)
        .into_iter()
        .filter(|event| batch.event_is_user_authored(event.id))
        .any(|event| {
            source::evidence_segments(&event.content)
                .into_iter()
                .any(|segment| {
                    has_user_context_framing(&segment)
                        && has_third_party_fact_token_support(&candidate.claim_text, &segment)
                })
        })
}

fn has_third_party_fact_token_support(claim_text: &str, source_text: &str) -> bool {
    let claim_tokens = third_party_fact_tokens(claim_text);
    if claim_tokens.len() < 2 {
        return false;
    }
    let source_tokens = short_support_tokens(source_text);
    if third_party_fact_is_negated(&claim_tokens, &source_tokens) {
        return false;
    }
    claim_tokens
        .iter()
        .all(|token| source_tokens.iter().any(|source| source == token))
}

fn third_party_fact_is_negated(claim_tokens: &[String], source_tokens: &[String]) -> bool {
    let positions = claim_tokens
        .iter()
        .filter_map(|claim| source_tokens.iter().position(|source| source == claim))
        .collect::<Vec<_>>();
    if positions.len() != claim_tokens.len() {
        return false;
    }
    let start = positions
        .iter()
        .min()
        .copied()
        .unwrap_or(0)
        .saturating_sub(1);
    let end = positions.iter().max().copied().unwrap_or(0);
    source_tokens[start..=end].iter().any(|token| {
        matches!(
            token.as_str(),
            "cannot" | "cant" | "doesn" | "don" | "isn" | "never" | "no" | "not"
        )
    })
}

fn third_party_fact_tokens(text: &str) -> Vec<String> {
    let mut tokens = short_support_tokens(text)
        .into_iter()
        .filter(|token| !is_third_party_fact_stopword(token))
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn is_third_party_fact_stopword(token: &str) -> bool {
    matches!(
        token,
        "a" | "an"
            | "and"
            | "as"
            | "for"
            | "from"
            | "her"
            | "his"
            | "i"
            | "in"
            | "is"
            | "me"
            | "my"
            | "of"
            | "on"
            | "our"
            | "s"
            | "the"
            | "their"
            | "to"
            | "user"
            | "we"
    )
}

fn has_user_context_framing(text: &str) -> bool {
    let tokens = short_support_tokens(text);
    has_user_reference_token(&tokens) && has_durable_third_party_context_token(&tokens)
}

fn has_user_reference_token(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "i" | "me" | "my" | "mine" | "our" | "ours" | "us" | "we"
        )
    }) || tokens.windows(2).any(|window| window == ["user", "s"])
}

fn has_durable_third_party_context_token(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "approver"
                | "client"
                | "colleague"
                | "collaborator"
                | "coworker"
                | "family"
                | "father"
                | "friend"
                | "husband"
                | "manager"
                | "mentor"
                | "mother"
                | "owner"
                | "partner"
                | "qa"
                | "release"
                | "repo"
                | "review"
                | "reviewer"
                | "sibling"
                | "spouse"
                | "stakeholder"
                | "team"
                | "teammate"
                | "vendor"
                | "wife"
                | "workflow"
        )
    })
}

fn is_supported_for_candidate_queue(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> bool {
    if candidate.source_kind == "inferred_from_behavior" {
        return has_behavior_source_evidence(candidate, batch);
    }
    is_supported_by_user_source_event(candidate, batch)
        || is_supported_negative_user_constraint(candidate, batch)
}

fn has_behavior_source_evidence(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> bool {
    batch
        .events_for_candidate(candidate)
        .into_iter()
        .any(|event| {
            source::is_behavior_source_event(event)
                && source::source_preview_for_event(event, candidate).is_some()
        })
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
            source::evidence_segments(&event.content)
                .into_iter()
                .any(|segment| {
                    let segment_text = normalize_support_text(&segment);
                    has_conservative_source_support(&candidate_text, &segment_text)
                        || short_variants
                            .iter()
                            .any(|variant| has_short_exact_source_support(variant, &segment_text))
                })
        })
}

fn is_supported_negative_user_constraint(
    candidate: &ParsedUserContextCandidate,
    batch: &CandidateSourceBatch,
) -> bool {
    if candidate.source_kind != "explicit_user_statement"
        || !matches!(
            candidate.claim_type,
            super::claims::UserContextClaimType::Preference
                | super::claims::UserContextClaimType::Constraint
        )
        || !contains_negative_constraint_token(&short_support_tokens(&candidate.claim_text))
    {
        return false;
    }
    let variants = short_user_context_support_variants(&candidate.claim_text);
    batch
        .events_for_candidate(candidate)
        .into_iter()
        .any(|event| {
            batch.event_is_user_authored(event.id)
                && source::evidence_segments(&event.content)
                    .into_iter()
                    .any(|segment| {
                        variants.iter().any(|variant| {
                            has_short_exact_source_support_allowing_risk(variant, &segment)
                        })
                    })
        })
}

fn contains_negative_constraint_token(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "avoid"
                | "cannot"
                | "cant"
                | "deny"
                | "disable"
                | "don"
                | "never"
                | "no"
                | "not"
                | "prevent"
                | "skip"
                | "without"
        )
    })
}

fn normalize_support_text(text: &str) -> String {
    let raw_tokens = text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut tokens = Vec::with_capacity(raw_tokens.len());
    let mut index = 0;
    while index < raw_tokens.len() {
        if raw_tokens.get(index).is_some_and(|token| token == "the")
            && raw_tokens
                .get(index + 1)
                .is_some_and(|token| token == "user")
            && raw_tokens.get(index + 2).is_some_and(|token| token == "s")
        {
            tokens.push("my".to_string());
            index += 3;
            continue;
        }
        if raw_tokens.get(index).is_some_and(|token| token == "user")
            && raw_tokens.get(index + 1).is_some_and(|token| token == "s")
        {
            tokens.push("my".to_string());
            index += 2;
            continue;
        }
        if raw_tokens.get(index).is_some_and(|token| token == "the")
            && raw_tokens
                .get(index + 1)
                .is_some_and(|token| token == "user")
        {
            tokens.push("i".to_string());
            index += 2;
            continue;
        }
        if raw_tokens.get(index).is_some_and(|token| token == "user") {
            tokens.push("i".to_string());
            index += 1;
            continue;
        }
        tokens.push(raw_tokens[index].clone());
        index += 1;
    }
    tokens.join(" ")
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

fn has_short_exact_source_support_allowing_risk(
    candidate_tokens: &[String],
    source_text: &str,
) -> bool {
    if !(3..=10).contains(&candidate_tokens.len()) {
        return false;
    }
    let source_tokens = short_support_tokens(source_text);
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
