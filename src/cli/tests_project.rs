use clap::{CommandFactory, Parser};

use super::project_types::{ProjectAction, ProjectAliasAction};
use super::types::{Cli, Commands};

#[test]
fn project_alias_commands_parse_and_appear_in_help() {
    let cli = Cli::try_parse_from([
        "remem",
        "project",
        "alias",
        "add",
        "/worktree",
        "--canonical",
        "/main",
        "--actor",
        "user",
        "--reason",
        "same project",
    ])
    .expect("parse alias preview");
    assert!(matches!(
        cli.command,
        Commands::Project {
            action: ProjectAction::Alias {
                action: ProjectAliasAction::Add { apply: false, .. }
            }
        }
    ));

    let cli = Cli::try_parse_from([
        "remem",
        "project",
        "alias",
        "revoke",
        "/worktree",
        "--actor",
        "user",
        "--reason",
        "retired",
        "--apply",
    ])
    .expect("parse alias revoke");
    assert!(matches!(
        cli.command,
        Commands::Project {
            action: ProjectAction::Alias {
                action: ProjectAliasAction::Revoke { apply: true, .. }
            }
        }
    ));
    assert!(Cli::command().render_help().to_string().contains("project"));
}
