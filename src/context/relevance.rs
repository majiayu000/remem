use std::collections::{HashMap, HashSet};

use crate::memory::lesson::LessonMemory;
use crate::memory::{Memory, MemoryType};

use super::types::{LoadedContext, SessionSummaryBrief};

pub(super) const SESSIONSTART_RELEVANCE_POLICY_VERSION: &str = "sessionstart_significant_token_v1";
pub(super) const BELOW_RELEVANCE_THRESHOLD: &str = "below_sessionstart_relevance_threshold";
pub(super) const SESSIONSTART_K_LIMIT: &str = "sessionstart_k_limit";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RelevanceSection {
    Lessons,
    MemoryIndex,
    Sessions,
}

#[derive(Debug, Clone)]
pub(super) struct RelevanceCandidate {
    pub stable_key: String,
    pub section: RelevanceSection,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct RelevanceDecision {
    pub score: f64,
    pub selected: bool,
    pub drop_reason: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub(super) struct SessionStartRelevancePlan {
    pub state: &'static str,
    pub k: usize,
    pub threshold: Option<f64>,
    pub candidate_count: usize,
    pub eligible_count: usize,
    pub selected_count: usize,
    pub below_threshold_count: usize,
    pub k_limited_count: usize,
    decisions: HashMap<String, RelevanceDecision>,
}

pub(super) struct GovernedContextInputs {
    pub lessons: Vec<LessonMemory>,
    pub memories: Vec<Memory>,
    pub summaries: Vec<SessionSummaryBrief>,
}

impl SessionStartRelevancePlan {
    pub fn disabled(candidates: &[RelevanceCandidate]) -> Self {
        let decisions = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.stable_key.clone(),
                    RelevanceDecision {
                        score: 0.0,
                        selected: true,
                        drop_reason: None,
                    },
                )
            })
            .collect();
        Self {
            state: "disabled",
            k: 0,
            threshold: None,
            candidate_count: candidates.len(),
            eligible_count: candidates.len(),
            selected_count: candidates.len(),
            below_threshold_count: 0,
            k_limited_count: 0,
            decisions,
        }
    }

    pub fn decision(&self, stable_key: &str) -> Option<RelevanceDecision> {
        self.decisions.get(stable_key).copied()
    }

    pub fn selected(&self, stable_key: &str) -> bool {
        self.decision(stable_key)
            .is_some_and(|decision| decision.selected)
    }

    pub fn provenance(&self) -> String {
        let threshold = self
            .threshold
            .map(|value| format!("{value:.6}"))
            .unwrap_or_else(|| "none".to_string());
        format!(
            "policy={};state={};k={};threshold={};candidates={};eligible={};selected={};below_threshold={};k_limited={}",
            SESSIONSTART_RELEVANCE_POLICY_VERSION,
            self.state,
            self.k,
            threshold,
            self.candidate_count,
            self.eligible_count,
            self.selected_count,
            self.below_threshold_count,
            self.k_limited_count
        )
    }
}

pub(super) fn candidates_for_loaded(
    loaded: &LoadedContext,
    core_ids: &HashSet<i64>,
) -> Vec<RelevanceCandidate> {
    let mut candidates = Vec::new();
    candidates.extend(loaded.lessons.iter().map(|lesson| RelevanceCandidate {
        stable_key: memory_stable_key(lesson.memory.id),
        section: RelevanceSection::Lessons,
        text: format!("{} {}", lesson.memory.title, lesson.memory.text),
    }));
    candidates.extend(
        loaded
            .memories
            .iter()
            .filter(|memory| !core_ids.contains(&memory.id))
            .filter(|memory| {
                MemoryType::parse(&memory.memory_type).is_none_or(MemoryType::is_indexed)
            })
            .map(|memory| RelevanceCandidate {
                stable_key: memory_stable_key(memory.id),
                section: RelevanceSection::MemoryIndex,
                text: format!("{} {}", memory.title, memory.text),
            }),
    );
    candidates.extend(loaded.summaries.iter().map(|summary| RelevanceCandidate {
        stable_key: session_stable_key(summary.id),
        section: RelevanceSection::Sessions,
        text: format!(
            "{} {}",
            summary.request,
            summary.completed.as_deref().unwrap_or_default()
        ),
    }));
    candidates
}

pub(super) fn selected_inputs(
    loaded: &LoadedContext,
    plan: &SessionStartRelevancePlan,
    core_ids: &HashSet<i64>,
) -> GovernedContextInputs {
    GovernedContextInputs {
        lessons: loaded
            .lessons
            .iter()
            .filter(|lesson| plan.selected(&memory_stable_key(lesson.memory.id)))
            .cloned()
            .collect(),
        memories: loaded
            .memories
            .iter()
            .filter(|memory| !core_ids.contains(&memory.id))
            .filter(|memory| plan.selected(&memory_stable_key(memory.id)))
            .cloned()
            .collect(),
        summaries: loaded
            .summaries
            .iter()
            .filter(|summary| plan.selected(&session_stable_key(summary.id)))
            .cloned()
            .collect(),
    }
}

pub(super) fn memory_stable_key(id: i64) -> String {
    format!("memory:{id}")
}

pub(super) fn session_stable_key(id: i64) -> String {
    format!("session_summary:{id}")
}

pub(super) fn build_sessionstart_relevance_plan(
    query: Option<&str>,
    k: usize,
    candidates: &[RelevanceCandidate],
) -> SessionStartRelevancePlan {
    if k == 0 {
        return SessionStartRelevancePlan::disabled(candidates);
    }

    let query_tokens = significant_tokens(query.unwrap_or_default());
    let mut scored = candidates
        .iter()
        .map(|candidate| {
            (
                candidate,
                relevance_score_from_tokens(&query_tokens, &candidate.text),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(
        |(left_candidate, left_score), (right_candidate, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left_candidate.section.cmp(&right_candidate.section))
                .then_with(|| left_candidate.stable_key.cmp(&right_candidate.stable_key))
        },
    );

    let positive_scores = scored
        .iter()
        .map(|(_, score)| *score)
        .filter(|score| *score > 0.0)
        .collect::<Vec<_>>();
    let threshold = derive_threshold(&positive_scores, k);
    let eligible_count = threshold.map_or(0, |threshold| {
        positive_scores
            .iter()
            .filter(|score| **score + f64::EPSILON >= threshold)
            .count()
    });

    let mut selected_remaining = k;
    let mut decisions = HashMap::with_capacity(scored.len());
    let mut below_threshold_count = 0usize;
    let mut k_limited_count = 0usize;
    for (candidate, score) in scored {
        let eligible = threshold.is_some_and(|threshold| score + f64::EPSILON >= threshold);
        let (selected, drop_reason) = if !eligible {
            below_threshold_count += 1;
            (false, Some(BELOW_RELEVANCE_THRESHOLD))
        } else if selected_remaining == 0 {
            k_limited_count += 1;
            (false, Some(SESSIONSTART_K_LIMIT))
        } else {
            selected_remaining -= 1;
            (true, None)
        };
        decisions.insert(
            candidate.stable_key.clone(),
            RelevanceDecision {
                score,
                selected,
                drop_reason,
            },
        );
    }
    let selected_count = k.saturating_sub(selected_remaining);
    SessionStartRelevancePlan {
        state: if selected_count == 0 {
            "blank"
        } else {
            "applied"
        },
        k,
        threshold,
        candidate_count: candidates.len(),
        eligible_count,
        selected_count,
        below_threshold_count,
        k_limited_count,
        decisions,
    }
}

pub(super) fn significant_token_relevance_score(query: &str, candidate_text: &str) -> f64 {
    relevance_score_from_tokens(&significant_tokens(query), candidate_text)
}

fn relevance_score_from_tokens(query_tokens: &HashSet<String>, candidate_text: &str) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let candidate_text = candidate_text.to_lowercase();
    let matched = query_tokens
        .iter()
        .filter(|token| candidate_text.contains(token.as_str()))
        .count();
    matched as f64 / query_tokens.len() as f64
}

fn derive_threshold(positive_scores: &[f64], k: usize) -> Option<f64> {
    if positive_scores.is_empty() || k == 0 {
        return None;
    }
    if positive_scores.len() < k {
        return positive_scores.last().copied();
    }
    let boundary = positive_scores[k - 1];
    let next_lower = positive_scores
        .iter()
        .skip(k)
        .copied()
        .find(|score| *score + f64::EPSILON < boundary);
    Some(next_lower.map_or(boundary, |next| (boundary + next) / 2.0))
}

fn significant_tokens(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| ch.is_ascii_punctuation())
                .to_lowercase()
        })
        .filter(|token| token.chars().count() >= 3 || !token.is_ascii())
        .filter(|token| !is_stop_token(token))
        .collect()
}

fn is_stop_token(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "should"
            | "when"
            | "where"
            | "what"
            | "why"
            | "how"
            | "into"
            | "was"
            | "were"
            | "does"
            | "need"
            | "about"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(key: &str, section: RelevanceSection, text: &str) -> RelevanceCandidate {
        RelevanceCandidate {
            stable_key: key.to_string(),
            section,
            text: text.to_string(),
        }
    }

    #[test]
    fn score_reuses_significant_token_overlap() {
        assert_eq!(
            significant_token_relevance_score(
                "How do we fix startup migration races?",
                "Startup migration locking fix"
            ),
            3.0 / 4.0
        );
        assert_eq!(
            significant_token_relevance_score("the and how", "unrelated"),
            0.0
        );
    }

    #[test]
    fn selector_is_stable_across_ties_and_sections() {
        let candidates = vec![
            candidate("session_summary:3", RelevanceSection::Sessions, "cache"),
            candidate("memory:2", RelevanceSection::MemoryIndex, "cache"),
            candidate("memory:1", RelevanceSection::Lessons, "cache"),
        ];
        let plan = build_sessionstart_relevance_plan(Some("cache"), 2, &candidates);

        assert_eq!(plan.state, "applied");
        assert_eq!(plan.threshold, Some(1.0));
        assert!(plan.selected("memory:1"));
        assert!(plan.selected("memory:2"));
        assert!(!plan.selected("session_summary:3"));
        assert_eq!(
            plan.decision("session_summary:3")
                .and_then(|decision| decision.drop_reason),
            Some(SESSIONSTART_K_LIMIT)
        );
    }

    #[test]
    fn selector_derives_gap_threshold_and_does_not_backfill() {
        let candidates = vec![
            candidate("memory:1", RelevanceSection::Lessons, "alpha beta"),
            candidate("memory:2", RelevanceSection::MemoryIndex, "alpha"),
            candidate("memory:3", RelevanceSection::Sessions, "unrelated"),
        ];
        let plan =
            build_sessionstart_relevance_plan(Some("alpha beta gamma delta"), 1, &candidates);

        assert_eq!(plan.threshold, Some(0.375));
        assert!(plan.selected("memory:1"));
        assert_eq!(plan.below_threshold_count, 2);
        assert_eq!(plan.k_limited_count, 0);
    }

    #[test]
    fn selector_handles_sparse_blank_and_disabled_modes() {
        let candidates = vec![
            candidate("memory:1", RelevanceSection::Lessons, "alpha"),
            candidate("memory:2", RelevanceSection::MemoryIndex, "unrelated"),
        ];
        let sparse = build_sessionstart_relevance_plan(Some("alpha beta"), 5, &candidates);
        assert_eq!(sparse.threshold, Some(0.5));
        assert_eq!(sparse.selected_count, 1);

        let blank = build_sessionstart_relevance_plan(Some("gamma"), 1, &candidates);
        assert_eq!(blank.state, "blank");
        assert_eq!(blank.selected_count, 0);
        assert_eq!(blank.below_threshold_count, 2);

        let disabled = build_sessionstart_relevance_plan(Some("gamma"), 0, &candidates);
        assert_eq!(disabled.state, "disabled");
        assert!(disabled.selected("memory:1"));
        assert!(disabled.selected("memory:2"));
    }
}
