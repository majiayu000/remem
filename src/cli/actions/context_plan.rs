//! `remem context-plan` (GH-934): compile and print a deterministic
//! task-aware retrieval plan. Debug/audit surface only: the output is
//! the plan (intent, channels, filters, budgets, policy version, reason
//! codes) and never memory contents. No database, LLM, or network access.

use anyhow::{bail, Result};

use crate::context_bundle::{
    AgentRole, ContextIntent, ContextRequest, ProjectRef, RiskClass, CONTEXT_BUNDLE_SCHEMA_VERSION,
};
use crate::retrieval_router::plan;

use super::super::context_types::ContextPlanArgs;
use super::super::cwd::resolve_cwd_arg;

pub(in crate::cli) fn run_context_plan(args: ContextPlanArgs) -> Result<()> {
    let explicit_intent = args.intent.as_deref().map(parse_intent).transpose()?;
    let project = match args.project {
        Some(key) => key,
        None => crate::db::project_from_cwd(&resolve_cwd_arg(args.cwd)),
    };
    let request = ContextRequest {
        schema_version: CONTEXT_BUNDLE_SCHEMA_VERSION,
        task: args.task,
        project: ProjectRef { key: project },
        branch: args.branch,
        worktree: None,
        role: parse_role(&args.role)?,
        as_of_epoch: args.as_of_epoch,
        token_budget: args.token_budget,
        risk: parse_risk(&args.risk)?,
        include_superseded: args.include_superseded,
    };
    let compiled = plan(&request, explicit_intent)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&compiled)?);
        return Ok(());
    }
    print_plan_summary(&compiled);
    Ok(())
}

fn print_plan_summary(plan: &crate::retrieval_router::RetrievalPlan) {
    println!("intent: {}", enum_name(&plan.intent));
    println!("intent_source: {}", enum_name(&plan.intent_source));
    println!("policy_version: {}", plan.policy_version);
    println!("plan_hash: {}", plan.plan_hash);
    println!(
        "role: {}  risk: {}",
        enum_name(&plan.role),
        enum_name(&plan.risk)
    );
    println!(
        "filters: project={} branch={} include_superseded={} as_of_epoch={}",
        plan.filters.project,
        plan.filters.branch.as_deref().unwrap_or("-"),
        plan.filters.include_superseded,
        plan.filters.as_of_epoch
    );
    println!("token_budget: {}", plan.token_budget);
    println!(
        "rerank: enabled={} pool={} k={} canonical_top1={}",
        plan.rerank_policy.enabled,
        plan.rerank_policy.candidate_pool,
        plan.rerank_policy.output_k,
        plan.rerank_policy.require_canonical_evidence_top1
    );
    println!(
        "trust: min={}  abstention: {}",
        enum_name(&plan.trust_policy.minimum_trust),
        enum_name(&plan.abstention_policy.mode)
    );
    println!("enabled channels (weight/limit/cap):");
    for cp in plan.channel_plans.iter().filter(|c| c.enabled) {
        println!(
            "  {:<22} {:.2} / {} / {}",
            cp.channel.name(),
            cp.weight,
            cp.candidate_limit,
            cp.max_contribution
        );
    }
    let disabled: Vec<&str> = plan
        .channel_plans
        .iter()
        .filter(|c| !c.enabled)
        .map(|c| c.channel.name())
        .collect();
    println!("disabled channels: {}", disabled.join(", "));
    if plan.output_sections.is_empty() {
        println!("output sections: none (ranked result list, no sections)");
    } else {
        println!("output sections (item limit/relevance-governed):");
        for section in &plan.output_sections {
            println!(
                "  {:<14} {} / {}",
                enum_name(&section.channel),
                section.item_limit,
                section.relevance_governed
            );
        }
    }
    println!(
        "section budgets (tokens): total={} preferences={} lessons={} core={} \
workstreams={} memory_index={} sessions={}",
        plan.section_budgets.total_tokens,
        plan.section_budgets.preferences,
        plan.section_budgets.lessons,
        plan.section_budgets.core,
        plan.section_budgets.workstreams,
        plan.section_budgets.memory_index,
        plan.section_budgets.sessions
    );
    println!("reason_codes: {}", plan.reason_codes.join(", "));
}

/// Render a serde snake_case enum value without a hand-kept name table.
fn enum_name<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn parse_intent(value: &str) -> Result<ContextIntent> {
    Ok(match value {
        "session-start" => ContextIntent::SessionStart,
        "resume-work" => ContextIntent::ResumeWork,
        "explain-decision" => ContextIntent::ExplainDecision,
        "debug-failure" => ContextIntent::DebugFailure,
        "apply-preference" => ContextIntent::ApplyPreference,
        "review-change" => ContextIntent::ReviewChange,
        "explore-history" => ContextIntent::ExploreHistory,
        other => bail!(
            "unknown intent {other:?}; expected session-start, resume-work, \
             explain-decision, debug-failure, apply-preference, review-change, \
             or explore-history"
        ),
    })
}

fn parse_role(value: &str) -> Result<AgentRole> {
    Ok(match value {
        "coder" => AgentRole::Coder,
        "reviewer" => AgentRole::Reviewer,
        "planner" => AgentRole::Planner,
        "researcher" => AgentRole::Researcher,
        other => bail!("unknown role {other:?}; expected coder, reviewer, planner, or researcher"),
    })
}

fn parse_risk(value: &str) -> Result<RiskClass> {
    Ok(match value {
        "low" => RiskClass::Low,
        "medium" => RiskClass::Medium,
        "high" => RiskClass::High,
        other => bail!("unknown risk {other:?}; expected low, medium, or high"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_strings_parse_to_router_intents() {
        assert_eq!(
            parse_intent("resume-work").unwrap(),
            ContextIntent::ResumeWork
        );
        assert_eq!(
            parse_intent("explain-decision").unwrap(),
            ContextIntent::ExplainDecision
        );
        assert_eq!(
            parse_intent("debug-failure").unwrap(),
            ContextIntent::DebugFailure
        );
        assert_eq!(
            parse_intent("apply-preference").unwrap(),
            ContextIntent::ApplyPreference
        );
        assert_eq!(
            parse_intent("review-change").unwrap(),
            ContextIntent::ReviewChange
        );
        assert_eq!(
            parse_intent("explore-history").unwrap(),
            ContextIntent::ExploreHistory
        );
        assert_eq!(
            parse_intent("session-start").unwrap(),
            ContextIntent::SessionStart
        );
        assert!(parse_intent("bogus").is_err());
    }

    #[test]
    fn role_and_risk_strings_parse() {
        assert_eq!(parse_role("reviewer").unwrap(), AgentRole::Reviewer);
        assert!(parse_role("owner").is_err());
        assert_eq!(parse_risk("high").unwrap(), RiskClass::High);
        assert!(parse_risk("extreme").is_err());
    }

    #[test]
    fn enum_name_renders_snake_case() {
        assert_eq!(
            enum_name(&ContextIntent::ExplainDecision),
            "explain_decision"
        );
        assert_eq!(enum_name(&RiskClass::High), "high");
    }
}
