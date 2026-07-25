use super::{
    procedure_types::ProcedureExportFormatArg,
    types::{Cli, Commands, ProcedureAction},
};
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

#[test]
fn cli_parses_procedures_export_guard_flags() {
    let cli = Cli::parse_from([
        "remem",
        "procedures",
        "export",
        "42",
        "--format",
        "claude-skill",
        "--out",
        "/tmp/remem-drafts",
        "--overwrite-generated",
    ]);

    match cli.command {
        Commands::Procedures {
            action:
                ProcedureAction::Export {
                    memory_id,
                    format,
                    out,
                    overwrite_generated,
                },
        } => {
            assert_eq!(memory_id, 42);
            assert_eq!(format, ProcedureExportFormatArg::ClaudeSkill);
            assert_eq!(
                out.as_deref(),
                Some(std::path::Path::new("/tmp/remem-drafts"))
            );
            assert!(overwrite_generated);
        }
        _ => panic!("expected procedures export command"),
    }
}
