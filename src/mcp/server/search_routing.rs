//! GH-934 task-aware routing support for the MCP `search` tool: compiles an
//! explicit RetrievalPlan from caller intent/role/risk/budget inputs and
//! reports the applied plan as audit metadata.
use serde::Serialize;

use super::super::types::SearchParams;
use super::errors::{McpToolError, McpToolResult};
use crate::memory::service;

pub(super) struct RoutedSearchPlan {
    pub(super) plan: crate::retrieval_router::RetrievalPlan,
    pub(super) policy: service::SearchRoutingPolicy,
    pub(super) applied_effects: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct SearchRetrievalPlanReport {
    schema_version: u32,
    policy_version: String,
    plan_hash: String,
    intent: String,
    intent_source: String,
    role: String,
    risk: String,
    reason_codes: Vec<String>,
    applied_effects: Vec<String>,
    filters: crate::context_bundle::ContextFilters,
    rerank_policy: crate::retrieval_router::RerankPolicy,
    enabled_channels: Vec<String>,
    disabled_channels: Vec<String>,
}

impl RoutedSearchPlan {
    pub(super) fn report(&self) -> SearchRetrievalPlanReport {
        SearchRetrievalPlanReport {
            schema_version: self.plan.schema_version,
            policy_version: self.plan.policy_version.clone(),
            plan_hash: self.plan.plan_hash.clone(),
            intent: enum_name(&self.plan.intent),
            intent_source: enum_name(&self.plan.intent_source),
            role: enum_name(&self.plan.role),
            risk: enum_name(&self.plan.risk),
            reason_codes: self.plan.reason_codes.clone(),
            applied_effects: self.applied_effects.clone(),
            filters: self.plan.filters.clone(),
            rerank_policy: self.plan.rerank_policy.clone(),
            enabled_channels: self
                .plan
                .channel_plans
                .iter()
                .filter(|channel| channel.enabled)
                .map(|channel| channel.channel.name().to_string())
                .collect(),
            disabled_channels: self
                .plan
                .channel_plans
                .iter()
                .filter(|channel| !channel.enabled)
                .map(|channel| channel.channel.name().to_string())
                .collect(),
        }
    }
}

pub(super) fn compile_search_retrieval_plan(
    tool: &'static str,
    params: &SearchParams,
) -> McpToolResult<Option<RoutedSearchPlan>> {
    let routing_requested = params.task_intent.is_some()
        || params.role.is_some()
        || params.risk.is_some()
        || params.token_budget.is_some()
        || params.include_superseded.is_some();
    if !routing_requested {
        return Ok(None);
    }
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| {
            McpToolError::invalid_request(
                tool,
                "task-aware search routing requires a non-empty query",
            )
        })?;
    let project = params
        .project
        .as_deref()
        .map(str::trim)
        .filter(|project| !project.is_empty())
        .ok_or_else(|| {
            McpToolError::invalid_request(
                tool,
                "task-aware search routing requires an explicit project",
            )
        })?;
    let token_budget = params.token_budget.unwrap_or(4_000);
    if token_budget == 0 {
        return Err(McpToolError::invalid_request(
            tool,
            "token_budget must be greater than zero",
        ));
    }
    let explicit_intent = params
        .task_intent
        .as_deref()
        .map(parse_search_intent)
        .transpose()?;
    let role = parse_search_role(tool, params.role.as_deref().unwrap_or("coder"))?;
    let risk = parse_search_risk(tool, params.risk.as_deref().unwrap_or("medium"))?;
    let request = crate::context_bundle::ContextRequest {
        schema_version: crate::context_bundle::CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: query.to_string(),
        project: crate::context_bundle::ProjectRef {
            key: project.to_string(),
        },
        branch: params.branch.clone(),
        worktree: None,
        role,
        as_of_epoch: 0,
        token_budget,
        risk,
        include_superseded: params.include_superseded.unwrap_or(false),
    };
    let plan = crate::retrieval_router::plan(&request, explicit_intent)
        .map_err(|error| McpToolError::invalid_request(tool, error.to_string()))?;
    let policy = service::SearchRoutingPolicy::from_retrieval_plan(&plan);
    let mut applied_effects = vec!["weights_from_retrieval_plan".to_string()];
    if policy.rerank_enabled {
        applied_effects.push("rerank_requested_by_intent".to_string());
    } else {
        applied_effects.push("rerank_skipped_by_intent".to_string());
    }
    if !policy.raw_fallback_enabled {
        applied_effects.push("raw_fallback_disabled_by_abstention_policy".to_string());
    }
    Ok(Some(RoutedSearchPlan {
        plan,
        policy,
        applied_effects,
    }))
}

fn parse_search_intent(raw: &str) -> McpToolResult<crate::context_bundle::ContextIntent> {
    let normalized = normalize_enum_arg(raw);
    match normalized.as_str() {
        "resume-work" => Ok(crate::context_bundle::ContextIntent::ResumeWork),
        "explain-decision" => Ok(crate::context_bundle::ContextIntent::ExplainDecision),
        "debug-failure" => Ok(crate::context_bundle::ContextIntent::DebugFailure),
        "apply-preference" => Ok(crate::context_bundle::ContextIntent::ApplyPreference),
        "review-change" => Ok(crate::context_bundle::ContextIntent::ReviewChange),
        "explore-history" => Ok(crate::context_bundle::ContextIntent::ExploreHistory),
        _ => Err(McpToolError::invalid_request(
            "search",
            "unknown task_intent; expected resume_work, explain_decision, debug_failure, apply_preference, review_change, or explore_history",
        )),
    }
}

fn parse_search_role(
    tool: &'static str,
    raw: &str,
) -> McpToolResult<crate::context_bundle::AgentRole> {
    match normalize_enum_arg(raw).as_str() {
        "coder" => Ok(crate::context_bundle::AgentRole::Coder),
        "reviewer" => Ok(crate::context_bundle::AgentRole::Reviewer),
        "planner" => Ok(crate::context_bundle::AgentRole::Planner),
        "researcher" => Ok(crate::context_bundle::AgentRole::Researcher),
        _ => Err(McpToolError::invalid_request(
            tool,
            "unknown role; expected coder, reviewer, planner, or researcher",
        )),
    }
}

fn parse_search_risk(
    tool: &'static str,
    raw: &str,
) -> McpToolResult<crate::context_bundle::RiskClass> {
    match normalize_enum_arg(raw).as_str() {
        "low" => Ok(crate::context_bundle::RiskClass::Low),
        "medium" => Ok(crate::context_bundle::RiskClass::Medium),
        "high" => Ok(crate::context_bundle::RiskClass::High),
        _ => Err(McpToolError::invalid_request(
            tool,
            "unknown risk; expected low, medium, or high",
        )),
    }
}

fn normalize_enum_arg(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn enum_name<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|json| json.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}
