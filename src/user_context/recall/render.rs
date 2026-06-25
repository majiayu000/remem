use std::collections::HashSet;

use anyhow::Result;

use super::normalize::compact_line;
use super::types::{
    NormalizedRequest, RecallCandidate, RecallState, UserRecallDiagnostics, UserRecallDroppedItem,
    UserRecallItem, UserRecallResult,
};

pub(super) fn finalize(
    normalized: NormalizedRequest,
    mut state: RecallState,
) -> Result<UserRecallResult> {
    dedup_candidates(&mut state.candidates);
    state.candidates.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.source_type.cmp(&right.source_type))
            .then_with(|| left.source_id.cmp(&right.source_id))
    });

    let (included, mut budget_drops, context, used_chars) =
        apply_budget(state.candidates, normalized.limit, normalized.budget_chars);
    state.dropped.append(&mut budget_drops);
    state.counts.dropped = state.dropped.len();

    Ok(UserRecallResult {
        query: normalized.query,
        project: normalized.project,
        task_intent: normalized.task_intent,
        host: normalized.host,
        empty: included.is_empty(),
        context,
        usage_policy: (!included.is_empty())
            .then_some(crate::user_context::usage_policy::USER_CONTEXT_USAGE_POLICY),
        included,
        dropped: state.dropped,
        diagnostics: UserRecallDiagnostics {
            requested_limit: normalized.limit,
            budget_chars: normalized.budget_chars,
            used_chars,
            candidate_counts: state.counts,
        },
    })
}

fn dedup_candidates(candidates: &mut Vec<RecallCandidate>) {
    let mut seen = HashSet::new();
    candidates.retain(|candidate| {
        seen.insert((
            candidate.source_type.clone(),
            candidate.source_id,
            candidate.title.clone().unwrap_or_default(),
        ))
    });
}

fn apply_budget(
    candidates: Vec<RecallCandidate>,
    limit: usize,
    budget_chars: usize,
) -> (
    Vec<UserRecallItem>,
    Vec<UserRecallDroppedItem>,
    String,
    usize,
) {
    let mut included = Vec::new();
    let mut dropped = Vec::new();
    let mut context_lines = Vec::new();
    let mut used = 0usize;

    for candidate in candidates {
        if included.len() >= limit {
            dropped.push(drop_for_candidate(candidate, "limit_exceeded"));
            continue;
        }
        let line = format_context_line(&candidate);
        let projected = used.saturating_add(line.chars().count()).saturating_add(1);
        if projected > budget_chars {
            dropped.push(drop_for_candidate(candidate, "budget_exceeded"));
            continue;
        }
        used = projected;
        context_lines.push(line);
        included.push(UserRecallItem {
            source_type: candidate.source_type,
            source_id: candidate.source_id,
            title: candidate.title,
            text: candidate.text,
            reason_codes: candidate.reason_codes,
            source_refs: candidate.source_refs,
        });
    }

    (included, dropped, context_lines.join("\n"), used)
}

fn format_context_line(candidate: &RecallCandidate) -> String {
    let label = match (candidate.source_type.as_str(), candidate.source_id) {
        (source, Some(id)) => format!("{source}:{id}"),
        (source, None) => source.to_string(),
    };
    match candidate.title.as_deref() {
        Some(title) if !title.trim().is_empty() => {
            format!(
                "- [{label}] {}: {}",
                compact_line(title, 80),
                candidate.text
            )
        }
        _ => format!("- [{label}] {}", candidate.text),
    }
}

fn drop_for_candidate(candidate: RecallCandidate, reason_code: &str) -> UserRecallDroppedItem {
    UserRecallDroppedItem {
        source_type: candidate.source_type,
        source_id: candidate.source_id,
        label: candidate.title,
        reason_code: reason_code.to_string(),
    }
}
