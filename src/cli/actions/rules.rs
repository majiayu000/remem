use std::io::Read;

use anyhow::Result;

use crate::cli::cwd::resolve_cwd_arg;
use crate::cli::types::{RuleActionArg, RuleHostArg, RulesAction};
use crate::db;
use crate::rules::{self, RuleAction, RulePredicate};

pub(in crate::cli) fn run_rules(action: RulesAction) -> Result<()> {
    let (project_arg, mutation) = match action {
        RulesAction::List { project } => (project, None),
        RulesAction::Disable { rule_id } => (None, Some(RuleMutation::Disabled(rule_id, true))),
        RulesAction::Enable { rule_id } => (None, Some(RuleMutation::Disabled(rule_id, false))),
        RulesAction::SetAction {
            rule_id,
            action,
            host,
        } => (
            None,
            Some(RuleMutation::Action(rule_id, action.into(), host)),
        ),
        RulesAction::Eval { host } => return run_rules_eval(host),
    };
    let project = db::project_from_cwd(&resolve_cwd_arg(project_arg));
    let data_dir = db::absolute_data_dir()?;

    if let Some(mutation) = mutation {
        let conn = db::open_db()?;
        let (rule_id, message) = match mutation {
            RuleMutation::Disabled(rule_id, disabled) => {
                rules::set_rule_disabled(&conn, &data_dir, &project, &rule_id, disabled)?;
                let state = if disabled { "disabled" } else { "enabled" };
                (rule_id, format!("Rule override saved: {state}"))
            }
            RuleMutation::Action(rule_id, action, host) => {
                rules::set_rule_action(
                    &conn,
                    &data_dir,
                    &project,
                    &rule_id,
                    action,
                    host == Some(RuleHostArg::ClaudeCode),
                )?;
                (
                    rule_id,
                    format!("Rule action override saved: {}", action_label(action)),
                )
            }
        };
        println!("{message} ({rule_id}); pending worker rebuild.");
        return Ok(());
    }

    let project_rules = rules::list_project_rules(&data_dir, &project)?;
    if project_rules.rules.is_empty() {
        println!("No compiled rules for project '{}'.", project_rules.project);
        return Ok(());
    }
    println!(
        "Compiled rules for '{}' (compiled_at_epoch={}):",
        project_rules.project, project_rules.compiled_at_epoch
    );
    for rule in project_rules.rules {
        let (predicate_kind, predicate_data) = predicate_display(&rule.predicate);
        let override_action = rule
            .override_state
            .action_override
            .map(action_label)
            .unwrap_or("default");
        println!(
            "  {} source_memory={} reinforcement={} predicate={} data={} base_action={} effective_action={} override={} disabled={}",
            rule.rule_id,
            rule.source_memory_id,
            rule.reinforcement_count,
            predicate_kind,
            predicate_data,
            action_label(rule.action),
            action_label(rule.effective_action()),
            override_action,
            rule.override_state.disabled
        );
    }
    Ok(())
}

enum RuleMutation {
    Disabled(String, bool),
    Action(String, RuleAction, Option<RuleHostArg>),
}

fn run_rules_eval(host: Option<RuleHostArg>) -> Result<()> {
    let data_dir = match db::absolute_data_dir() {
        Ok(data_dir) => data_dir,
        Err(error) => {
            crate::log::error(
                "rules-eval",
                &format!("resolve remem data directory: {error:#}"),
            );
            return Ok(());
        }
    };
    let mut raw = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut raw) {
        rules::log_evaluation_error_once_with_diagnostic(
            &data_dir,
            None,
            None,
            &[rules::EvaluationDiagnosticCode::HookInputRead],
            &format!("read Claude PreToolUse hook input: {error}"),
        );
        return Ok(());
    }
    let session_hint = rules::session_id_hint(&raw);
    let project_hint = rules::project_hint(&raw);
    let config = match crate::runtime_config::rule_compilation_config() {
        Ok(config) => config,
        Err(error) => {
            rules::log_evaluation_error_once_with_diagnostic(
                &data_dir,
                session_hint.as_deref(),
                None,
                &[rules::EvaluationDiagnosticCode::Config],
                &format!("read rule compilation config: {error:#}"),
            );
            return Ok(());
        }
    };
    let evaluated = match rules::evaluate_pre_tool_use_with_diagnostics(
        &raw,
        host.map(rule_host_label),
        &data_dir,
        config.enabled,
    ) {
        Ok(evaluated) => evaluated,
        Err(error) => {
            rules::log_evaluation_error_once_with_diagnostic(
                &data_dir,
                session_hint.as_deref(),
                project_hint.as_deref(),
                &[rules::EvaluationDiagnosticCode::HookInput],
                &format!("{error:#}"),
            );
            return Ok(());
        }
    };
    if !evaluated.evaluation.diagnostics.is_empty() {
        rules::log_evaluation_error_once_with_diagnostic(
            &data_dir,
            evaluated.evaluation.session_id.as_deref(),
            evaluated.project.as_deref(),
            &evaluated.diagnostic_codes,
            &evaluated.evaluation.diagnostics.join("; "),
        );
        return Ok(());
    }
    if let Some(output) = evaluated.evaluation.output {
        match serde_json::to_string(&output) {
            Ok(output) => println!("{output}"),
            Err(error) => rules::log_evaluation_error_once_with_diagnostic(
                &data_dir,
                evaluated.evaluation.session_id.as_deref(),
                evaluated.project.as_deref(),
                &[rules::EvaluationDiagnosticCode::OutputSerialize],
                &format!("serialize Claude PreToolUse hook output: {error}"),
            ),
        }
    }
    Ok(())
}

fn rule_host_label(host: RuleHostArg) -> &'static str {
    match host {
        RuleHostArg::ClaudeCode => crate::runtime_config::CLAUDE_HOST,
        RuleHostArg::CodexCli => crate::runtime_config::CODEX_HOST,
    }
}

impl From<RuleActionArg> for RuleAction {
    fn from(value: RuleActionArg) -> Self {
        match value {
            RuleActionArg::Warn => Self::Warn,
            RuleActionArg::Block => Self::Block,
        }
    }
}

fn action_label(action: RuleAction) -> &'static str {
    match action {
        RuleAction::Warn => "warn",
        RuleAction::Block => "block",
    }
}

fn predicate_display(predicate: &RulePredicate) -> (&'static str, String) {
    match predicate {
        RulePredicate::CommandRegex { pattern, .. } => ("command_regex", format!("{pattern:?}")),
        RulePredicate::CommitTrailerForbidden { trailer, .. } => {
            ("commit_trailer_forbidden", format!("{trailer:?}"))
        }
        RulePredicate::GitPushForceForbidden { .. } => (
            "git_push_force_forbidden",
            "git push force option".to_string(),
        ),
    }
}
