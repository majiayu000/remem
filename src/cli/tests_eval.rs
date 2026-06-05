use super::types::{Cli, Commands};
use clap::Parser;

#[test]
fn cli_parses_eval_json_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval",
        "--dataset",
        "fixtures/golden.json",
        "--json",
        "-k",
        "3",
    ]);

    match cli.command {
        Commands::Eval { dataset, k, json } => {
            assert_eq!(dataset, "fixtures/golden.json");
            assert_eq!(k, 3);
            assert!(json);
        }
        _ => panic!("expected eval command"),
    }
}
