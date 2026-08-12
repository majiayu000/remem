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
//! v1 started as a debug-only compiler exposed through
//! `remem context-plan --task ... --json`. MCP `search` now has the first
//! production wiring slice: callers may pass an explicit task intent / role /
//! risk / budget, and the compiled plan selects search weights, graph
//! expansion, rerank participation, and raw-fallback abstention. Intent
//! resolution remains fully deterministic: explicit caller intent wins, simple
//! keyword rules are the only fallback, and unclassifiable tasks conservatively
//! fall back to `ExploreHistory` with the generic policy. No LLM or network
//! call is ever made by the router itself. Full per-channel evidence loaders,
//! generated-enrichment execution, default-on eval gates, and golden-fixture
//! ablation remain follow-up work on GH-934.

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
pub(crate) use planner::{plan_context_bundle_with_limits, plan_session_start_with_limits};
