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

#[test]
fn cli_parses_eval_extraction_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-extraction",
        "--corpus",
        "fixtures/extraction.json",
        "--baseline",
        "fixtures/baseline.json",
        "--json",
        "--check-baseline",
    ]);

    match cli.command {
        Commands::EvalExtraction(args) => {
            assert_eq!(args.corpus, "fixtures/extraction.json");
            assert_eq!(args.baseline, "fixtures/baseline.json");
            assert!(args.json);
            assert!(args.check_baseline);
        }
        _ => panic!("expected eval-extraction command"),
    }
}
