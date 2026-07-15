use anyhow::Result;

use crate::cli::cwd::resolve_cwd_arg;
use crate::cli::types::{RuleActionArg, RulesAction};
use crate::db;
use crate::rules::{self, RuleAction, RulePredicate};

pub(in crate::cli) fn run_rules(action: RulesAction) -> Result<()> {
    let (project_arg, mutation) = match action {
        RulesAction::List { project } => (project, None),
        RulesAction::Disable { rule_id } => (None, Some(RuleMutation::Disabled(rule_id, true))),
        RulesAction::Enable { rule_id } => (None, Some(RuleMutation::Disabled(rule_id, false))),
        RulesAction::SetAction { rule_id, action } => {
            (None, Some(RuleMutation::Action(rule_id, action.into())))
        }
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
            RuleMutation::Action(rule_id, action) => {
                rules::set_rule_action(&conn, &data_dir, &project, &rule_id, action)?;
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
    Action(String, RuleAction),
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
    }
}
