use super::types::{Cli, Commands, RuleActionArg, RulesAction};
use clap::Parser;

#[test]
fn cli_parses_compiled_rule_management_commands() {
    let list = Cli::parse_from(["remem", "rules", "list", "--project", "/tmp/remem"]);
    match list.command {
        Commands::Rules {
            action: RulesAction::List { project },
        } => assert_eq!(project.as_deref(), Some("/tmp/remem")),
        _ => panic!("expected rules list command"),
    }

    let disable = Cli::parse_from(["remem", "rules", "disable", "pref-1-1"]);
    assert!(matches!(
        disable.command,
        Commands::Rules {
            action: RulesAction::Disable { rule_id }
        } if rule_id == "pref-1-1"
    ));

    let enable = Cli::parse_from(["remem", "rules", "enable", "pref-1-1"]);
    assert!(matches!(
        enable.command,
        Commands::Rules {
            action: RulesAction::Enable { rule_id }
        } if rule_id == "pref-1-1"
    ));

    for action_name in ["warn", "block"] {
        let set_action = Cli::parse_from(["remem", "rules", "set-action", "pref-1-1", action_name]);
        assert!(matches!(
            set_action.command,
            Commands::Rules {
                action: RulesAction::SetAction {
                    rule_id,
                    action: RuleActionArg::Warn | RuleActionArg::Block,
                }
            } if rule_id == "pref-1-1"
        ));
    }
}
