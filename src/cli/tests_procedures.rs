use super::types::{Cli, Commands, ProcedureAction};
use clap::Parser;

#[test]
fn cli_parses_procedures_list_filters() {
    let cli = Cli::parse_from([
        "remem",
        "procedures",
        "list",
        "--project",
        "/tmp/remem",
        "--limit",
        "5",
        "--offset",
        "10",
        "--json",
    ]);

    match cli.command {
        Commands::Procedures {
            action:
                ProcedureAction::List {
                    project,
                    limit,
                    offset,
                    json,
                },
        } => {
            assert_eq!(project.as_deref(), Some("/tmp/remem"));
            assert_eq!(limit, 5);
            assert_eq!(offset, 10);
            assert!(json);
        }
        _ => panic!("expected procedures list command"),
    }
}
