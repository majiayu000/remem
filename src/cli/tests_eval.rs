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

#[test]
fn cli_parses_eval_gates_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-gates",
        "--baseline",
        "fixtures/baseline.json",
        "--thresholds",
        "fixtures/thresholds.json",
        "--golden-dataset",
        "fixtures/golden.json",
        "--json-out",
        "/tmp/eval-gates.json",
    ]);

    match cli.command {
        Commands::EvalGates(args) => {
            assert_eq!(args.baseline, "fixtures/baseline.json");
            assert_eq!(args.thresholds, "fixtures/thresholds.json");
            assert_eq!(args.golden_dataset, "fixtures/golden.json");
            assert_eq!(args.json_out.as_deref(), Some("/tmp/eval-gates.json"));
        }
        _ => panic!("expected eval-gates command"),
    }
}

#[test]
fn cli_parses_eval_graph_decision_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-graph-decision",
        "--dataset",
        "fixtures/golden.json",
        "--json-out",
        "/tmp/graph-decision.json",
        "--json",
        "-k",
        "7",
    ]);

    match cli.command {
        Commands::EvalGraphDecision(args) => {
            assert_eq!(args.dataset, "fixtures/golden.json");
            assert_eq!(args.json_out, "/tmp/graph-decision.json");
            assert_eq!(args.k, 7);
            assert!(args.json);
        }
        _ => panic!("expected eval-graph-decision command"),
    }
}

#[test]
fn cli_parses_eval_weight_grid_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-weight-grid",
        "--dataset",
        "fixtures/golden.json",
        "--json-out",
        "/tmp/weight-grid.json",
        "--json",
        "-k",
        "7",
    ]);

    match cli.command {
        Commands::EvalWeightGrid(args) => {
            assert_eq!(args.dataset, "fixtures/golden.json");
            assert_eq!(args.json_out, "/tmp/weight-grid.json");
            assert_eq!(args.k, 7);
            assert!(args.json);
        }
        _ => panic!("expected eval-weight-grid command"),
    }
}
