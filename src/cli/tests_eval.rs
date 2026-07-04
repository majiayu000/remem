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
fn cli_parses_bench_verify_options() {
    let cli = Cli::parse_from([
        "remem",
        "bench",
        "verify",
        "--root",
        "eval/public",
        "--json-out",
        "/tmp/remem-bench-verify.json",
    ]);

    match cli.command {
        Commands::Bench { action } => match action {
            super::eval_types::BenchAction::Verify(args) => {
                assert_eq!(args.root, "eval/public");
                assert_eq!(args.json_out, "/tmp/remem-bench-verify.json");
            }
            _ => panic!("expected bench verify action"),
        },
        _ => panic!("expected bench verify command"),
    }
}

#[test]
fn cli_parses_bench_memory_options() {
    let cli = Cli::parse_from([
        "remem",
        "bench",
        "memory",
        "--suite",
        "remem-code-memory",
        "--condition",
        "remem_default",
        "--root",
        "eval/public",
        "--artifact-prefix",
        "memory/artifacts/remem-code-memory-v1",
        "--json-out",
        "/tmp/remem-code-memory.json",
    ]);

    match cli.command {
        Commands::Bench { action } => match action {
            super::eval_types::BenchAction::Memory(args) => {
                assert_eq!(args.suite, "remem-code-memory");
                assert_eq!(args.condition.as_deref(), Some("remem_default"));
                assert_eq!(args.root, "eval/public");
                assert_eq!(
                    args.artifact_prefix.as_deref(),
                    Some("memory/artifacts/remem-code-memory-v1")
                );
                assert_eq!(args.json_out, "/tmp/remem-code-memory.json");
            }
            _ => panic!("expected bench memory action"),
        },
        _ => panic!("expected bench memory command"),
    }
}

#[test]
fn cli_parses_bench_coding_options() {
    let cli = Cli::parse_from([
        "remem",
        "bench",
        "coding",
        "--suite",
        "issue385-v1",
        "--task-set",
        "smoke",
        "--dry-run",
        "--json-out",
        "/tmp/remem-issue385-v1-dry-run.json",
    ]);

    match cli.command {
        Commands::Bench { action } => match action {
            super::eval_types::BenchAction::Coding(args) => {
                assert_eq!(args.suite, "issue385-v1");
                assert_eq!(args.task_set, "smoke");
                assert!(args.dry_run);
                assert_eq!(args.runs_per_condition, 3);
                assert_eq!(args.json_out, "/tmp/remem-issue385-v1-dry-run.json");
            }
            _ => panic!("expected bench coding action"),
        },
        _ => panic!("expected bench coding command"),
    }
}

#[test]
fn cli_parses_bench_report_options() {
    let cli = Cli::parse_from([
        "remem",
        "bench",
        "report",
        "--root",
        "eval/public",
        "--json-out",
        "eval/public/reports/baseline.json",
        "--markdown-out",
        "eval/public/reports/baseline.md",
    ]);

    match cli.command {
        Commands::Bench { action } => match action {
            super::eval_types::BenchAction::Report(args) => {
                assert_eq!(args.root, "eval/public");
                assert_eq!(args.json_out, "eval/public/reports/baseline.json");
                assert_eq!(args.markdown_out, "eval/public/reports/baseline.md");
            }
            _ => panic!("expected bench report action"),
        },
        _ => panic!("expected bench report command"),
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
fn cli_parses_eval_provider_comparison_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-provider-comparison",
        "--dataset",
        "fixtures/golden.json",
        "--json-out",
        "/tmp/provider-comparison.json",
        "--json",
        "--allow-api",
        "-k",
        "7",
    ]);

    match cli.command {
        Commands::EvalProviderComparison(args) => {
            assert_eq!(args.dataset, "fixtures/golden.json");
            assert_eq!(args.json_out, "/tmp/provider-comparison.json");
            assert_eq!(args.k, 7);
            assert!(args.json);
            assert!(args.allow_api);
        }
        _ => panic!("expected eval-provider-comparison command"),
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
fn cli_parses_eval_capacity_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-capacity",
        "--dataset",
        "fixtures/golden.json",
        "--seed",
        "17",
        "--scales",
        "1,10",
        "--json-out",
        "/tmp/capacity.json",
        "--json",
        "-k",
        "7",
    ]);

    match cli.command {
        Commands::EvalCapacity(args) => {
            assert_eq!(args.dataset, "fixtures/golden.json");
            assert_eq!(args.seed, 17);
            assert_eq!(args.scales, "1,10");
            assert_eq!(args.json_out.as_deref(), Some("/tmp/capacity.json"));
            assert_eq!(args.k, 7);
            assert!(args.json);
        }
        _ => panic!("expected eval-capacity command"),
    }
}

#[test]
fn cli_parses_eval_associative_baseline_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-associative-baseline",
        "--dataset",
        "fixtures/golden.json",
        "--json-out",
        "/tmp/associative-baseline.json",
        "--json",
        "-k",
        "7",
    ]);

    match cli.command {
        Commands::EvalAssociativeBaseline(args) => {
            assert_eq!(args.dataset, "fixtures/golden.json");
            assert_eq!(args.json_out, "/tmp/associative-baseline.json");
            assert_eq!(args.k, 7);
            assert!(args.json);
        }
        _ => panic!("expected eval-associative-baseline command"),
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

#[test]
fn cli_parses_eval_coding_bench_options() {
    let cli = Cli::parse_from([
        "remem",
        "eval-coding-bench",
        "--fixture",
        "eval/coding-bench/fixtures/tasks.json",
        "--runs-per-condition",
        "3",
        "--json-out",
        "eval/coding-bench/reports/baseline.json",
        "--condition",
        "remem",
        "--task",
        "slug-normalizer-contract",
        "--runner",
        "codex",
        "--codex-bin",
        "/usr/bin/false",
        "--model",
        "gpt-5.5",
        "--provider",
        "codexapi",
        "--reasoning-effort",
        "medium",
        "--ignore-budget",
        "--keep-workdirs",
    ]);

    match cli.command {
        Commands::EvalCodingBench(args) => {
            assert_eq!(args.fixture, "eval/coding-bench/fixtures/tasks.json");
            assert_eq!(args.runs_per_condition, 3);
            assert_eq!(
                args.json_out.as_deref(),
                Some("eval/coding-bench/reports/baseline.json")
            );
            assert_eq!(args.condition.as_deref(), Some("remem"));
            assert_eq!(args.task.as_deref(), Some("slug-normalizer-contract"));
            assert_eq!(args.task_set, "full");
            assert_eq!(args.runner, "codex");
            assert_eq!(args.codex_bin, "/usr/bin/false");
            assert_eq!(args.model, "gpt-5.5");
            assert_eq!(args.provider.as_deref(), Some("codexapi"));
            assert_eq!(args.reasoning_effort, "medium");
            assert!(args.ignore_budget);
            assert!(args.keep_workdirs);
        }
        _ => panic!("expected eval-coding-bench command"),
    }
}
