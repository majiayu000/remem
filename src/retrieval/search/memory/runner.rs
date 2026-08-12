use anyhow::Result;
use rusqlite::Connection;

use crate::memory::Memory;

use super::listing::search_without_query;
use super::text::{search_with_query, search_with_query_explain};
use super::SearchExplain;
use super::SearchExplainDetails;
use super::SearchWeights;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchExecutionPolicy {
    pub(crate) weights: SearchWeights,
    pub(crate) rerank: SearchRerankPolicy,
    pub(crate) disable_zero_weight_channels: bool,
}

impl SearchExecutionPolicy {
    pub(crate) fn production() -> Self {
        Self {
            weights: SearchWeights::production(),
            rerank: SearchRerankPolicy::Ambient,
            disable_zero_weight_channels: false,
        }
    }

    pub(crate) fn explain_default() -> Self {
        Self {
            weights: SearchWeights::default(),
            rerank: SearchRerankPolicy::Ambient,
            disable_zero_weight_channels: false,
        }
    }

    pub(crate) fn with_weights(weights: SearchWeights) -> Self {
        Self {
            weights,
            rerank: SearchRerankPolicy::Ambient,
            disable_zero_weight_channels: false,
        }
    }

    pub(crate) fn routed(
        weights: SearchWeights,
        rerank_enabled: bool,
        candidate_pool: u32,
        output_k: u32,
    ) -> Self {
        Self {
            weights,
            rerank: SearchRerankPolicy::Routed {
                enabled: rerank_enabled,
                candidate_pool: candidate_pool as usize,
                output_k: output_k as usize,
            },
            disable_zero_weight_channels: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SearchRerankPolicy {
    /// Legacy behavior: let the ambient reranker config decide whether the
    /// post-fusion stage participates.
    Ambient,
    /// GH-934 routed behavior: the compiled RetrievalPlan decides whether the
    /// stage is requested and what top-N/top-k bounds it uses.
    Routed {
        enabled: bool,
        candidate_pool: usize,
        output_k: usize,
    },
}

pub fn search(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
) -> Result<Vec<Memory>> {
    search_with_branch_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        None,
        false,
    )
}

pub fn search_with_branch(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
) -> Result<Vec<Memory>> {
    search_with_branch_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn search_with_branch_with_suppressed_policy(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
) -> Result<Vec<Memory>> {
    let (memories, _) = search_with_branch_execution_policy_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        include_suppressed,
        false,
        SearchExecutionPolicy::production(),
    )?;
    Ok(memories)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_with_branch_execution_policy_with_suppressed_policy(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
    explain: bool,
    execution_policy: SearchExecutionPolicy,
) -> Result<(Vec<Memory>, Option<SearchExplainDetails>)> {
    execution_policy.weights.validate()?;
    match query {
        Some(query_text) if !query_text.is_empty() => {
            if explain {
                search_with_query_explain(
                    conn,
                    query_text,
                    project,
                    memory_type,
                    limit,
                    offset,
                    include_stale,
                    branch,
                    include_suppressed,
                    execution_policy,
                )
                .map(|result| (result.memories, Some(result.explain_details)))
            } else {
                search_with_query(
                    conn,
                    query_text,
                    project,
                    memory_type,
                    limit,
                    offset,
                    include_stale,
                    branch,
                    include_suppressed,
                    execution_policy,
                )
                .map(|memories| (memories, None))
            }
        }
        _ => search_without_query(
            conn,
            project,
            memory_type,
            limit,
            offset,
            include_stale,
            branch,
            include_suppressed,
        )
        .map(|memories| (memories, None)),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_with_branch_weights(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    weights: SearchWeights,
) -> Result<Vec<Memory>> {
    let (memories, _) = search_with_branch_execution_policy_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        false,
        false,
        SearchExecutionPolicy::with_weights(weights),
    )?;
    Ok(memories)
}

pub fn search_with_branch_explain(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
) -> Result<(Vec<Memory>, Option<SearchExplain>)> {
    search_with_branch_explain_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn search_with_branch_explain_details(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
) -> Result<(Vec<Memory>, Option<SearchExplainDetails>)> {
    search_with_branch_explain_details_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn search_with_branch_explain_with_suppressed_policy(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
) -> Result<(Vec<Memory>, Option<SearchExplain>)> {
    search_with_branch_explain_details_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        include_suppressed,
    )
    .map(|(memories, details)| (memories, details.map(|details| details.explain)))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_with_branch_explain_details_with_suppressed_policy(
    conn: &Connection,
    query: Option<&str>,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    include_stale: bool,
    branch: Option<&str>,
    include_suppressed: bool,
) -> Result<(Vec<Memory>, Option<SearchExplainDetails>)> {
    search_with_branch_execution_policy_with_suppressed_policy(
        conn,
        query,
        project,
        memory_type,
        limit,
        offset,
        include_stale,
        branch,
        include_suppressed,
        true,
        SearchExecutionPolicy::explain_default(),
    )
}
