use std::collections::{BTreeMap, BTreeSet};

use super::types::{MemoryBenchCondition, MemoryBenchEvidence, MemoryBenchTask};

const TOP_K: usize = 5;
const TRUNCATED_CONTEXT_ITEMS: usize = 1;

pub(super) fn fixture_retrieval_indices(
    condition: MemoryBenchCondition,
    task: &MemoryBenchTask,
) -> Option<Vec<usize>> {
    let indices = match condition {
        MemoryBenchCondition::NoMemory => Vec::new(),
        MemoryBenchCondition::TruncatedFullContext => recent_allowed(task, TRUNCATED_CONTEXT_ITEMS),
        MemoryBenchCondition::OracleEvidence => gold_supporting(task),
        MemoryBenchCondition::CompleteStoredMemory => allowed_evidence(task),
        MemoryBenchCondition::Bm25Baseline => rank_by_bm25(task),
        MemoryBenchCondition::VectorBaseline => rank_by_vector_proxy(task),
        MemoryBenchCondition::HybridRagBaseline => rank_by_hybrid_rag(task),
        MemoryBenchCondition::SummaryBaseline => active_summary_evidence(task),
        MemoryBenchCondition::RetrievedMemory | MemoryBenchCondition::RememDefault => {
            return None;
        }
    };
    Some(indices)
}

fn gold_supporting(task: &MemoryBenchTask) -> Vec<usize> {
    task.evidence
        .iter()
        .enumerate()
        .filter(|(_, evidence)| task.gold_supporting_event_ids.contains(&evidence.event_id))
        .map(|(idx, _)| idx)
        .collect()
}

fn allowed_evidence(task: &MemoryBenchTask) -> Vec<usize> {
    task.evidence
        .iter()
        .enumerate()
        .filter(|(_, evidence)| evidence.retention_allowed)
        .map(|(idx, _)| idx)
        .collect()
}

fn recent_allowed(task: &MemoryBenchTask, limit: usize) -> Vec<usize> {
    let mut scored = task
        .evidence
        .iter()
        .enumerate()
        .filter(|(_, evidence)| evidence.retention_allowed)
        .map(|(idx, evidence)| (idx, evidence.created_at_epoch.unwrap_or_default()))
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    scored.into_iter().take(limit).map(|(idx, _)| idx).collect()
}

fn active_summary_evidence(task: &MemoryBenchTask) -> Vec<usize> {
    task.evidence
        .iter()
        .enumerate()
        .filter(|(_, evidence)| evidence.retention_allowed && evidence.status == "active")
        .map(|(idx, _)| idx)
        .take(TOP_K)
        .collect()
}

fn rank_by_bm25(task: &MemoryBenchTask) -> Vec<usize> {
    rank_allowed(task, |evidence| lexical_score(&task.query, evidence))
}

fn rank_by_vector_proxy(task: &MemoryBenchTask) -> Vec<usize> {
    rank_allowed(task, |evidence| vector_proxy_score(&task.query, evidence))
}

fn rank_by_hybrid_rag(task: &MemoryBenchTask) -> Vec<usize> {
    let bm25 = ranked_scores(task, |evidence| lexical_score(&task.query, evidence));
    let vector = ranked_scores(task, |evidence| vector_proxy_score(&task.query, evidence));
    let mut fused: BTreeMap<usize, f64> = BTreeMap::new();
    for (rank, (idx, _)) in bm25.iter().enumerate() {
        *fused.entry(*idx).or_default() += rrf_score(rank);
    }
    for (rank, (idx, _)) in vector.iter().enumerate() {
        *fused.entry(*idx).or_default() += rrf_score(rank);
    }
    let mut ranked = fused.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked.into_iter().take(TOP_K).map(|(idx, _)| idx).collect()
}

fn rrf_score(rank: usize) -> f64 {
    1.0 / (60.0 + rank as f64 + 1.0)
}

fn rank_allowed(task: &MemoryBenchTask, score: impl Fn(&MemoryBenchEvidence) -> f64) -> Vec<usize> {
    ranked_scores(task, score)
        .into_iter()
        .take(TOP_K)
        .map(|(idx, _)| idx)
        .collect()
}

fn ranked_scores(
    task: &MemoryBenchTask,
    score: impl Fn(&MemoryBenchEvidence) -> f64,
) -> Vec<(usize, f64)> {
    let mut scored = task
        .evidence
        .iter()
        .enumerate()
        .filter(|(_, evidence)| evidence.retention_allowed)
        .map(|(idx, evidence)| {
            let status_bonus = if evidence.status == "active" {
                0.25
            } else {
                0.0
            };
            (idx, score(evidence) + status_bonus)
        })
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    scored
}

fn lexical_score(query: &str, evidence: &MemoryBenchEvidence) -> f64 {
    let query_tokens = tokens(query);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let title_tokens = tokens(&evidence.title);
    let content_tokens = tokens(&evidence.content);
    let title_hits = query_tokens
        .iter()
        .filter(|token| title_tokens.contains(*token))
        .count();
    let content_hits = query_tokens
        .iter()
        .filter(|token| content_tokens.contains(*token))
        .count();
    (title_hits as f64 * 2.0) + content_hits as f64
}

fn vector_proxy_score(query: &str, evidence: &MemoryBenchEvidence) -> f64 {
    let query_grams = char_trigrams(query);
    if query_grams.is_empty() {
        return 0.0;
    }
    let mut document_grams = char_trigrams(&evidence.title);
    document_grams.extend(char_trigrams(&evidence.content));
    let intersection = query_grams.intersection(&document_grams).count();
    let union = query_grams.union(&document_grams).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn tokens(value: &str) -> BTreeSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let normalized = token.trim().to_ascii_lowercase();
            (normalized.len() >= 2).then_some(normalized)
        })
        .collect()
}

fn char_trigrams(value: &str) -> BTreeSet<String> {
    let chars = value
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<Vec<_>>();
    chars
        .windows(3)
        .map(|window| window.iter().collect::<String>())
        .collect()
}
