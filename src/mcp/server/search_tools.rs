use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router};
use serde::Serialize;

use super::super::types::{CurrentStateParams, RawSearchHit, SearchParams, SearchResult};
use super::errors::{self, McpToolError, McpToolResult};
use super::MemoryServer;
use crate::memory::service;

const RAW_PREVIEW_CHARS: usize = 300;

struct RoutedSearchPlan {
    plan: crate::retrieval_router::RetrievalPlan,
    policy: service::SearchRoutingPolicy,
    applied_effects: Vec<String>,
}

#[derive(Serialize)]
struct SearchRetrievalPlanReport {
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
    fn report(&self) -> SearchRetrievalPlanReport {
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

#[tool_router(router = tool_router_search, vis = "pub(super)")]
impl MemoryServer {
    #[tool(
        description = "Read-only. Resolve one stable state_key to a JSON object with status=current, no_current, not_found, ambiguous, or unresolved_conflict, plus answer, compact history, and why edges. state_key must be non-blank; project/owner/type/as_of filters narrow the resolution. Use this instead of search when the durable key is known; use timeline for chronological observation context. Invalid input or database failures return a tool error."
    )]
    pub(super) fn current_state(
        &self,
        Parameters(params): Parameters<CurrentStateParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "current_state";
        if params.state_key.trim().is_empty() {
            return Err(McpToolError::invalid_request(TOOL, "state_key is required"));
        }
        self.with_conn(TOOL, |conn| {
            let req = service::CurrentStateRequest {
                state_key: params.state_key.clone(),
                project: params.project.clone(),
                owner_scope: params.owner_scope.clone(),
                owner_key: params.owner_key.clone(),
                memory_type: params.r#type.clone(),
                as_of_epoch: params.as_of_epoch,
                include_history: true,
            };
            let result = service::current_state(conn, &req).map_err(|e| {
                crate::log::warn("mcp", &format!("current_state failed: {}", e));
                McpToolError::db_query(TOOL, e)
            })?;
            errors::to_json_pretty(TOOL, &result)
        })
    }

    #[tool(
        description = "Read-only. Search or list curated memories: query is optional for standard search, while project/type/branch and visibility flags filter results. Optional task_intent/role/risk/token_budget/include_superseded compiles a GH-934 RetrievalPlan, applies its search/rerank/fallback policy, and returns retrieval_plan audit metadata. Returns a compact JSON object with results, source='memory', pagination, and next_step for get_observations(ids, source); limit defaults to 20 and offset to 0. Use current_state when an exact stable state_key is known, timeline for chronological observation context, and search_raw for literal chat recall. explain and multi_hop each require a non-blank query, and explain cannot be combined with multi_hop=true. Invalid combinations or curated-search database failures return a tool error; an automatic raw-archive fallback failure preserves the curated results and adds raw_hits_error to the successful response."
    )]
    pub(super) fn search(
        &self,
        Parameters(params): Parameters<SearchParams>,
    ) -> McpToolResult<String> {
        const TOOL: &str = "search";
        let start = std::time::Instant::now();
        let mut requested_multi_hop = params.multi_hop.unwrap_or(false);
        let requested_explain = params.explain.unwrap_or(false);
        if requested_multi_hop
            && params
                .query
                .as_deref()
                .is_none_or(|query| query.trim().is_empty())
        {
            return Err(McpToolError::invalid_request(
                TOOL,
                "multi_hop requires a non-empty query; set query or multi_hop=false",
            ));
        }
        if requested_explain
            && params
                .query
                .as_deref()
                .is_none_or(|query| query.trim().is_empty())
        {
            return Err(McpToolError::invalid_request(
                TOOL,
                "explain requires a non-empty query; set query or explain=false",
            ));
        }
        if requested_multi_hop && requested_explain {
            return Err(McpToolError::invalid_request(
                TOOL,
                "explain is not supported with multi_hop search yet; set multi_hop=false or explain=false",
            ));
        }
        let mut routed = compile_search_retrieval_plan(TOOL, &params)?;
        if let Some(route) = routed.as_mut() {
            if requested_explain && route.policy.use_multi_hop {
                route.policy.use_multi_hop = false;
                route
                    .applied_effects
                    .push("graph_expansion_not_applied_explain_requested".to_string());
            } else if params.multi_hop == Some(false) && route.policy.use_multi_hop {
                route.policy.use_multi_hop = false;
                route
                    .applied_effects
                    .push("graph_expansion_not_applied_multi_hop_explicit_false".to_string());
            } else if params.multi_hop.is_none() && route.policy.use_multi_hop {
                requested_multi_hop = true;
                route
                    .applied_effects
                    .push("graph_expansion_enabled_multi_hop".to_string());
            }
        }
        let routing_policy = routed.as_ref().map(|route| &route.policy);
        let retrieval_plan_report = routed.as_ref().map(RoutedSearchPlan::report);
        crate::log::info(
            "mcp",
            &format!(
                "search called query={:?} project={:?} type={:?} branch={:?} multi_hop={} limit={} offset={} routed={}",
                params.query,
                params.project,
                params.r#type,
                params.branch,
                requested_multi_hop,
                params.limit.unwrap_or(20),
                params.offset.unwrap_or(0),
                retrieval_plan_report.is_some(),
            ),
        );
        self.with_conn(TOOL, |conn| {
            let req = service::SearchRequest {
                query: params.query.clone(),
                project: params.project.clone(),
                memory_type: params.r#type.clone(),
                limit: params.limit.unwrap_or(20),
                offset: params.offset.unwrap_or(0),
                include_stale: params
                    .include_stale
                    .unwrap_or_else(service::default_include_stale),
                include_suppressed: params
                    .include_suppressed
                    .unwrap_or_else(service::default_include_suppressed),
                branch: params.branch.clone(),
                multi_hop: requested_multi_hop,
                explain: requested_explain,
            };
            let detailed =
                service::search_memories_with_explain_details_with_routing(
                    conn,
                    &req,
                    routing_policy,
                )
                .map_err(|e| {
                    crate::log::warn("mcp", &format!("search failed: {}", e));
                    McpToolError::db_query(TOOL, e)
                })?;
            let search_set = detailed.result;
            let req_limit = req.limit;
            let req_offset = req.offset;
            let service::SearchResultSet {
                memories,
                multi_hop,
                has_more,
                explain: _,
                raw_hits,
                raw_error,
            } = search_set;
            let staleness_labels = if requested_explain {
                let now_epoch = chrono::Utc::now().timestamp();
                crate::memory::staleness::memory_staleness_labels_for_memories(
                    conn,
                    &memories,
                    now_epoch,
                )
                .map_err(|error| {
                    crate::log::error(
                        "mcp",
                        &format!("search staleness source-anchor lookup failed: {error}"),
                    );
                    McpToolError::db_query(TOOL, error)
                })?
            } else {
                std::collections::HashMap::new()
            };

            let search_results: Vec<SearchResult> = memories
                .into_iter()
                .map(|memory| {
                    let staleness = requested_explain
                        .then(|| staleness_labels.get(&memory.id).cloned())
                        .flatten();
                    let updated = chrono::DateTime::from_timestamp(memory.updated_at_epoch, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default();
                    let temporal_facts = temporal_fact_preview_labels(&memory.text);
                    let preview = memory.text.chars().take(300).collect::<String>();
                    SearchResult {
                        id: memory.id,
                        r#type: memory.memory_type,
                        title: memory.title,
                        topic_key: memory.topic_key,
                        preview: Some(preview),
                        temporal_facts,
                        source: "memory".to_string(),
                        source_type: "memory".to_string(),
                        updated_at: updated,
                        project: memory.project,
                        status: memory.status,
                        staleness,
                    }
                })
                .collect();

            let raw_hits_json: Vec<RawSearchHit> = raw_hits
                .into_iter()
                .map(|msg| RawSearchHit {
                    id: msg.id,
                    source_type: "raw_archive".to_string(),
                    session_id: msg.session_id,
                    project: msg.project,
                    role: msg.role,
                    preview: msg.content.chars().take(RAW_PREVIEW_CHARS).collect(),
                    source: msg.source,
                    branch: msg.branch,
                    created_at: chrono::DateTime::from_timestamp(msg.created_at_epoch, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_default(),
                })
                .collect();

            let hop_info = if let Some(meta) = &multi_hop {
                format!(
                    " hops={} entities_discovered={}",
                    meta.hops,
                    meta.entities_discovered.len()
                )
            } else {
                String::new()
            };
            crate::log::info(
                "mcp",
                &format!(
                    "search done count={} raw_fallback={} {}ms{}",
                    search_results.len(),
                    raw_hits_json.len(),
                    start.elapsed().as_millis(),
                    hop_info,
                ),
            );

            let result_ids: Vec<i64> = search_results.iter().map(|result| result.id).collect();
            let next_offset = has_more.then_some(req_offset + req_limit);
            let mut response = serde_json::json!({
                "mode": "compact",
                "results": search_results,
                "next_step": {
                    "tool": "get_observations",
                    "source": "memory",
                    "ids": result_ids,
                    "reason": "Pass selected compact result IDs with source='memory' to fetch full details."
                },
                "pagination": {
                    "limit": req_limit,
                    "offset": req_offset,
                    "has_more": has_more,
                    "next_offset": next_offset,
                }
            });
            if req.include_suppressed {
                response["next_step"]["include_suppressed"] = serde_json::json!(true);
            }
            if let Some(meta) = multi_hop {
                response["multi_hop"] = serde_json::json!({
                    "hops": meta.hops,
                    "entities_discovered": meta.entities_discovered,
                });
            }
            if !raw_hits_json.is_empty() {
                response["raw_hits"] =
                    errors::to_json_value(TOOL, &raw_hits_json)?;
                response["raw_hits_note"] = serde_json::Value::String(
                    "raw_hits are source_type='raw_archive' chat rows, not curated memories; use search_raw for literal recall."
                        .to_string(),
                );
            }
            if let Some(error) = raw_error {
                response["raw_hits_error"] = serde_json::Value::String(error);
            }
            if has_more {
                response["has_more"] = serde_json::Value::Bool(true);
                response["next_offset"] = serde_json::Value::from(req_offset + req_limit);
            }
            if let Some(explain_details) = detailed.explain_details {
                response["explain"] = errors::to_json_value(TOOL, &explain_details)?;
            }
            if let Some(report) = retrieval_plan_report.as_ref() {
                response["retrieval_plan"] = errors::to_json_value(TOOL, report)?;
            }
            errors::to_json_pretty(TOOL, &response)
        })
    }
}

fn compile_search_retrieval_plan(
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

fn temporal_fact_preview_labels(text: &str) -> Vec<String> {
    text.lines()
        .next()
        .and_then(|line| line.strip_prefix("Temporal facts: "))
        .map(|line| {
            line.split("; ")
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use rmcp::handler::server::wrapper::Parameters;
    use serde_json::Value;

    use super::*;
    use crate::db::test_support::ScopedTestDataDir;
    use crate::mcp::types::SearchParams;
    use crate::memory;

    fn base_search_params(explain: Option<bool>) -> SearchParams {
        SearchParams {
            query: Some("aurora".to_string()),
            limit: Some(5),
            project: Some("/repo".to_string()),
            r#type: None,
            offset: Some(0),
            include_stale: Some(true),
            include_suppressed: None,
            branch: None,
            multi_hop: Some(false),
            explain,
            task_intent: None,
            role: None,
            risk: None,
            token_budget: None,
            include_superseded: None,
        }
    }

    fn default_visibility_search_params() -> SearchParams {
        SearchParams {
            include_stale: None,
            ..base_search_params(None)
        }
    }

    fn multi_hop_explain_params() -> SearchParams {
        SearchParams {
            multi_hop: Some(true),
            explain: Some(true),
            ..base_search_params(None)
        }
    }

    #[test]
    fn search_emits_explain_only_when_requested() -> Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-search-explain");
        let conn = crate::db::open_db()?;
        let memory_id = memory::insert_memory(
            &conn,
            Some("session-1"),
            "/repo",
            Some("aurora-contract"),
            "Aurora contract decision",
            "The aurora recall contract keeps search compact before expansion.",
            "decision",
            None,
        )?;
        drop(conn);

        let server = MemoryServer::new()?;
        let default_response = server
            .search(Parameters(base_search_params(None)))
            .map_err(anyhow::Error::msg)?;
        let default_json: Value = serde_json::from_str(&default_response)?;
        assert!(default_json.get("explain").is_none());
        assert!(default_json["results"][0].get("staleness").is_none());

        let explain_response = server
            .search(Parameters(base_search_params(Some(true))))
            .map_err(anyhow::Error::msg)?;
        let explain_json: Value = serde_json::from_str(&explain_response)?;

        assert_eq!(explain_json["results"][0]["id"], memory_id);
        assert_eq!(explain_json["results"][0]["staleness"]["status"], "active");
        assert_eq!(
            explain_json["results"][0]["staleness"]["source_anchor"],
            "untracked"
        );
        assert_eq!(explain_json["explain"]["query"], "aurora");
        assert_eq!(
            explain_json["explain"]["results"][0]["memory_id"],
            memory_id
        );
        assert_eq!(
            explain_json["explain"]["results"][0]["staleness"]["status"],
            "active"
        );
        let explain_result = &explain_json["explain"]["results"][0];
        let final_score = explain_result["final_score"]
            .as_f64()
            .expect("explain final_score should be numeric");
        let fusion_score = explain_result["fusion_score"]
            .as_f64()
            .expect("MCP explain should serialize fusion_score");
        let post_fusion_score_factor = explain_result["post_fusion_score_factor"]
            .as_f64()
            .expect("MCP explain should serialize post_fusion_score_factor");
        assert!((final_score - fusion_score * post_fusion_score_factor).abs() < 1e-12);
        let breakdown = &explain_json["explain"]["contribution_breakdowns"][0];
        assert_eq!(breakdown["memory_id"], memory_id);
        for contribution in breakdown["contributions"]
            .as_array()
            .context("MCP contribution breakdowns should be an array")?
        {
            let weight = contribution["weight"].as_f64().context("weight")?;
            let reciprocal_rank = contribution["reciprocal_rank"]
                .as_f64()
                .context("reciprocal_rank")?;
            let normalized_signal = contribution
                .get("normalized_signal")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let total = contribution["total_score"].as_f64().context("total")?;
            assert!((total - weight * reciprocal_rank * (1.0 + normalized_signal)).abs() < 1e-12);
        }
        Ok(())
    }

    #[test]
    fn search_explain_fails_when_source_anchor_label_fails() -> Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-search-staleness-source-error");
        let conn = crate::db::open_db()?;
        let memory_id = memory::insert_memory(
            &conn,
            Some("session-bad-staleness"),
            "/repo",
            None,
            "Aurora bad staleness",
            "The aurora bad source-anchor fixture should fail explain.",
            "decision",
            None,
        )?;
        conn.execute(
            "UPDATE memories SET files = '[not-json' WHERE id = ?1",
            [memory_id],
        )?;
        drop(conn);

        let server = MemoryServer::new()?;
        let error = server
            .search(Parameters(base_search_params(Some(true))))
            .expect_err("source-anchor failure should reject explain search");

        assert!(
            format!("{error:?}").contains("source-anchor staleness"),
            "{error:?}"
        );
        Ok(())
    }

    #[test]
    fn search_exposes_raw_fallback_failure() -> Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-search-raw-fallback-error");
        let conn = crate::db::open_db()?;
        conn.execute("DROP TABLE raw_messages_fts", [])?;
        drop(conn);

        let server = MemoryServer::new()?;
        let response = server
            .search(Parameters(base_search_params(None)))
            .map_err(anyhow::Error::msg)?;
        let json: Value = serde_json::from_str(&response)?;

        assert_eq!(json["results"].as_array().map(Vec::len), Some(0));
        assert!(json.get("raw_hits").is_none());
        assert!(json["raw_hits_error"]
            .as_str()
            .is_some_and(|error| error.contains("raw archive fallback failed")));
        Ok(())
    }

    #[test]
    fn search_hides_inactive_memories_by_default() -> Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-search-default-active");
        let conn = crate::db::open_db()?;
        let active_id = memory::insert_memory(
            &conn,
            Some("session-active"),
            "/repo",
            Some("aurora-active"),
            "Aurora active memory",
            "The aurora active decision remains visible.",
            "decision",
            None,
        )?;
        let stale_id = memory::insert_memory(
            &conn,
            Some("session-stale"),
            "/repo",
            Some("aurora-stale"),
            "Aurora stale memory",
            "The aurora stale decision is hidden by default.",
            "decision",
            None,
        )?;
        let archived_id = memory::insert_memory(
            &conn,
            Some("session-archived"),
            "/repo",
            Some("aurora-archived"),
            "Aurora archived memory",
            "The aurora archived decision is hidden by default.",
            "decision",
            None,
        )?;
        conn.execute(
            "UPDATE memories SET status = 'stale' WHERE id = ?1",
            rusqlite::params![stale_id],
        )?;
        conn.execute(
            "UPDATE memories SET status = 'archived' WHERE id = ?1",
            rusqlite::params![archived_id],
        )?;
        drop(conn);

        let server = MemoryServer::new()?;
        let response = server
            .search(Parameters(default_visibility_search_params()))
            .map_err(anyhow::Error::msg)?;
        let json: Value = serde_json::from_str(&response)?;

        assert_eq!(json["results"].as_array().map(Vec::len), Some(1));
        assert_eq!(json["results"][0]["id"], active_id);
        Ok(())
    }

    #[test]
    fn search_rejects_multi_hop_explain() -> Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-search-explain-multi-hop");
        let server = MemoryServer::new()?;

        let err = server
            .search(Parameters(multi_hop_explain_params()))
            .expect_err("multi-hop explain should be rejected");
        let json: Value = serde_json::from_str(&err.to_string())?;

        assert_eq!(json["error"]["code"], "invalid_request");
        assert!(
            json["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("multi_hop"),
            "{}",
            json
        );
        Ok(())
    }

    #[test]
    fn search_rejects_explain_without_query() -> Result<()> {
        let _dir = ScopedTestDataDir::new("mcp-search-explain-missing-query");
        let server = MemoryServer::new()?;

        for query in [None, Some("")] {
            let mut params = base_search_params(Some(true));
            params.query = query.map(str::to_string);

            let err = server
                .search(Parameters(params))
                .expect_err("queryless explain should be rejected");
            let json: Value = serde_json::from_str(&err.to_string())?;

            assert_eq!(json["error"]["code"], "invalid_request");
            assert!(
                json["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("non-empty query"),
                "{}",
                json
            );
        }
        Ok(())
    }

    #[test]
    fn temporal_fact_preview_labels_extracts_compact_labels() {
        let labels = temporal_fact_preview_labels(
            "Temporal facts: HarborMint verified_by Toma Reed; HarborMint blocked_by North\nBody",
        );

        assert_eq!(
            labels,
            vec![
                "HarborMint verified_by Toma Reed".to_string(),
                "HarborMint blocked_by North".to_string()
            ]
        );
    }
}
