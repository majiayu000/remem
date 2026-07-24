//! Retrieval Router v1 (GH-934): task-aware, deterministic plan compilation.
//!
//! Different tasks need different evidence: "why not async-trait" wants
//! decisions, superseded history, and git evidence rather than pure
//! semantic neighbors; a high-risk code change must not share freshness /
//! trust / abstention policy with casual history browsing. The router
//! turns a [`crate::context_bundle::ContextRequest`] plus an optional
//! explicit intent into a versioned [`RetrievalPlan`]:
//!
//! ```text
//! ContextRequest -> intent resolution -> RetrievalPlan
//!   (channel plans / filters / policies / budgets, stable plan hash)
//! ```
//!
//! v1 scope is plan compilation only, exposed through
//! `remem context-plan --task ... --json`. Intent resolution is fully
//! deterministic: explicit caller intent wins, simple keyword rules are
//! the only fallback, and unclassifiable tasks conservatively fall back
//! to `ExploreHistory` with the generic policy. No LLM or network call
//! is ever made. Wiring the plan into retrieval execution, the rerank
//! implementation (GH-851), graph expansion (GH-853), enrichment workers
//! (GH-850/928), and golden-fixture ablation are follow-up work on
//! GH-934.

mod domain;
mod intent;
mod planner;
#[cfg(test)]
mod tests;

pub use domain::{
    AbstentionMode, AbstentionPolicy, ChannelDegradation, ChannelPlan, FreshnessPolicy,
    IntentSource, RerankFallback, RerankPolicy, ResolvedIntent, RetrievalChannel, RetrievalPlan,
    TrustPolicy, RETRIEVAL_PLAN_SCHEMA_VERSION,
};
pub use intent::resolve_intent;
pub use planner::{plan, RETRIEVAL_ROUTER_POLICY_VERSION};
