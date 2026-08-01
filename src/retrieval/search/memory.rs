mod claim;
mod explain;
mod listing;
#[cfg(test)]
mod provider_fallback_tests;
mod runner;
mod source_anchor;
mod suppression_filter;
#[cfg(test)]
mod tests;
mod text;
pub(crate) mod usage_rank;
mod weights;

pub use explain::{
    ChannelContribution, ChannelContributionBreakdown, ChannelHit, SearchExplain,
    SearchExplainChannel, SearchExplainDetails, SearchExplainResult, SearchExplainResultBreakdown,
};
pub use runner::{
    search, search_with_branch, search_with_branch_explain, search_with_branch_explain_details,
    search_with_branch_explain_with_suppressed_policy, search_with_branch_with_suppressed_policy,
};
pub(crate) use runner::{
    search_with_branch_explain_details_with_suppressed_policy, search_with_branch_weights,
};
pub(crate) use source_anchor::apply_score_demotions;
pub(crate) use weights::SearchWeights;
