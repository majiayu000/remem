//! Deterministic intent resolution for the Retrieval Router (GH-934).
//!
//! Precedence: explicit caller intent > keyword fallback > conservative
//! `ExploreHistory` default. No LLM, no network, no clock: the same
//! inputs always resolve to the same intent, and keyword rules can only
//! pick an intent — they can never widen scope, lower trust thresholds,
//! or bypass abstention (those live in the deterministic planner).

use crate::context_bundle::ContextIntent;

use super::domain::{IntentSource, ResolvedIntent};

pub(super) const REASON_EXPLICIT_INTENT: &str = "explicit_intent";
pub(super) const REASON_SESSION_START_NOT_ROUTABLE: &str = "session_start_not_routable";
pub(super) const REASON_KEYWORD_MATCH_PREFIX: &str = "keyword_match_";
pub(super) const REASON_UNCLASSIFIED_FALLBACK: &str = "unclassified_conservative_fallback";

/// Keyword rules checked in fixed order; the first matching rule wins.
/// Matching is case-insensitive substring over the task text. Debug/
/// failure evidence outranks decision archaeology because misrouting a
/// failure investigation is costlier than misrouting a history browse.
const KEYWORD_RULES: [(ContextIntent, &str, &[&str]); 5] = [
    (
        ContextIntent::DebugFailure,
        "debug_failure",
        &[
            "debug",
            "error",
            "panic",
            "stack trace",
            "traceback",
            "regression",
            "broken",
            "failing",
            "flaky",
            "报错",
            "崩溃",
            "调试",
            "排查",
        ],
    ),
    (
        ContextIntent::ExplainDecision,
        "explain_decision",
        &[
            "why",
            "decision",
            "decided",
            "rationale",
            "trade-off",
            "tradeoff",
            "chose",
            "为什么",
            "决定",
            "决策",
        ],
    ),
    (
        ContextIntent::ReviewChange,
        "review_change",
        &[
            "review",
            "pull request",
            " pr ",
            "diff",
            "merge this",
            "审查",
            "评审",
        ],
    ),
    (
        ContextIntent::ResumeWork,
        "resume_work",
        &[
            "resume",
            "continue",
            "pick up where",
            "where were we",
            "last session",
            "next step",
            "继续",
            "上次",
            "接着",
        ],
    ),
    (
        ContextIntent::ApplyPreference,
        "apply_preference",
        &[
            "preference",
            "convention",
            "coding style",
            "style guide",
            "formatting rule",
            "偏好",
            "规范",
            "习惯",
        ],
    ),
];

/// Resolve the routing intent for a request.
///
/// - `Some(intent)` other than `SessionStart` is honored as-is;
/// - explicit `SessionStart` is not a router intent and conservatively
///   falls back to `ExploreHistory` with its own reason code;
/// - with no explicit intent, keyword rules run in fixed order;
/// - anything unclassifiable falls back to `ExploreHistory`.
pub fn resolve_intent(explicit: Option<ContextIntent>, task: &str) -> ResolvedIntent {
    match explicit {
        Some(ContextIntent::SessionStart) => ResolvedIntent {
            intent: ContextIntent::ExploreHistory,
            source: IntentSource::DefaultFallback,
            reason_code: REASON_SESSION_START_NOT_ROUTABLE.to_string(),
        },
        Some(intent) => ResolvedIntent {
            intent,
            source: IntentSource::Explicit,
            reason_code: REASON_EXPLICIT_INTENT.to_string(),
        },
        None => resolve_from_keywords(task),
    }
}

fn resolve_from_keywords(task: &str) -> ResolvedIntent {
    // Pad so word-boundary patterns like " pr " can match at the edges.
    let haystack = format!(" {} ", task.to_lowercase());
    for (intent, code, keywords) in KEYWORD_RULES {
        if keywords.iter().any(|kw| haystack.contains(kw)) {
            return ResolvedIntent {
                intent,
                source: IntentSource::KeywordFallback,
                reason_code: format!("{REASON_KEYWORD_MATCH_PREFIX}{code}"),
            };
        }
    }
    ResolvedIntent {
        intent: ContextIntent::ExploreHistory,
        source: IntentSource::DefaultFallback,
        reason_code: REASON_UNCLASSIFIED_FALLBACK.to_string(),
    }
}
