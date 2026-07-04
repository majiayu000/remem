use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::db;

use crate::cli::eval_types::{
    BenchAction, BenchCodingArgs, EvalCapacityArgs, EvalCodingBenchArgs, EvalProviderComparisonArgs,
};

pub(in crate::cli) fn run_bench(action: BenchAction) -> Result<()> {
    match action {
        BenchAction::Verify(args) => run_bench_verify(&args.root, &args.json_out),
        BenchAction::Memory(args) => run_bench_memory(
            &args.suite,
            args.condition.as_deref(),
            &args.root,
            args.artifact_prefix.as_deref(),
            &args.json_out,
        ),
        BenchAction::Coding(args) => run_bench_coding(args),
        BenchAction::Report(args) => {
            let report = crate::eval::bench_artifact::write_public_baseline_report(
                crate::eval::bench_artifact::BenchReportOptions {
                    root: Path::new(&args.root).to_path_buf(),
                    json_out: Path::new(&args.json_out).to_path_buf(),
                    markdown_out: Path::new(&args.markdown_out).to_path_buf(),
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
    }
}

fn run_bench_verify(root: &str, json_out: &str) -> Result<()> {
    let report = crate::eval::bench_artifact::verify_benchmark_artifacts(
        crate::eval::bench_artifact::BenchVerifyOptions {
            root: Path::new(root).to_path_buf(),
        },
    )?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = Path::new(json_out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "create benchmark verification report directory {}",
                    parent.display()
                )
            })?;
        }
    }
    fs::write(json_out, &report_json)
        .with_context(|| format!("write benchmark verification report {json_out}"))?;
    println!("{report_json}");
    if !report.passed {
        bail!(
            "bench verify failed with {} issue(s)",
            report.failures.len()
        );
    }
    Ok(())
}

fn run_bench_memory(
    suite: &str,
    condition: Option<&str>,
    root: &str,
    artifact_prefix: Option<&str>,
    json_out: &str,
) -> Result<()> {
    let report = crate::eval::memory_bench::run_memory_bench(
        crate::eval::memory_bench::MemoryBenchOptions {
            suite: suite.to_string(),
            condition: condition.map(str::to_string),
            json_out: json_out.to_string(),
            root: root.to_string(),
            artifact_prefix: artifact_prefix.map(str::to_string),
        },
    )?;
    let report_json = serde_json::to_string_pretty(&report)?;
    println!("{report_json}");
    Ok(())
}

fn run_bench_coding(args: BenchCodingArgs) -> Result<()> {
    let fixture = args
        .fixture
        .map(Ok)
        .unwrap_or_else(|| coding_fixture_for_suite(&args.suite))?;
    run_coding_bench_options(crate::eval::coding_bench::CodingBenchOptions {
        fixture_path: fixture,
        runs_per_condition: args.runs_per_condition,
        json_out: args.json_out,
        condition: args.condition,
        task: args.task,
        task_set: args.task_set,
        keep_workdirs: args.keep_workdirs,
        dry_run: args.dry_run,
        runner: args.runner,
        codex_bin: args.codex_bin,
        model: args.model,
        provider: args.provider,
        reasoning_effort: args.reasoning_effort,
        ignore_budget: args.ignore_budget,
    })
}

fn coding_fixture_for_suite(suite: &str) -> Result<String> {
    match suite {
        "issue385-v1" => Ok("eval/coding-bench/fixtures/tasks.json".to_string()),
        _ => bail!("unknown coding benchmark suite: {suite}"),
    }
}

pub(in crate::cli) fn run_eval_local() -> Result<()> {
    let conn = db::open_db()?;
    let report = crate::eval::local::run_eval(&conn)?;
    print!("{}", report);
    Ok(())
}

pub(in crate::cli) fn run_eval(dataset_path: &str, k: usize, json: bool) -> Result<()> {
    let dataset = crate::eval::golden::load_dataset(dataset_path)?;
    let report = if dataset.has_fixture_corpus() {
        crate::eval::golden::evaluate_dataset_with_fixture_corpus(&dataset, k)?
    } else {
        let conn = db::open_db()?;
        crate::eval::golden::evaluate_dataset(&conn, &dataset, k)?
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report);
    }
    Ok(())
}

pub(in crate::cli) async fn run_eval_e2e(k: usize, json: bool, keep_data_dir: bool) -> Result<()> {
    let report =
        crate::eval::e2e::run_sandbox_eval(crate::eval::e2e::E2eEvalOptions { k, keep_data_dir })
            .await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report);
    }
    Ok(())
}

pub(in crate::cli) fn run_eval_governance(k: usize, json: bool) -> Result<()> {
    let report = crate::eval::governance::run_sandbox_eval(
        crate::eval::governance::GovernanceEvalOptions { k },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report);
    }
    if !report.metrics.all_checks_passed {
        bail!("eval-governance checks failed");
    }
    Ok(())
}

pub(in crate::cli) fn run_eval_extraction(
    corpus_path: &str,
    baseline_path: &str,
    json: bool,
    check_baseline: bool,
) -> Result<()> {
    let report =
        crate::eval::extraction::run_corpus_path(crate::eval::extraction::ExtractionEvalOptions {
            corpus_path: corpus_path.to_string(),
        })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", report);
    }
    if check_baseline {
        let actual = serde_json::to_value(&report)?;
        let baseline_content = fs::read_to_string(baseline_path)
            .with_context(|| format!("read extraction eval baseline {baseline_path}"))?;
        let expected: serde_json::Value = serde_json::from_str(&baseline_content)
            .with_context(|| format!("parse extraction eval baseline {baseline_path}"))?;
        if actual != expected {
            bail!("extraction eval baseline changed; regenerate {baseline_path} intentionally");
        }
    }
    if !report.metrics.all_checks_passed {
        bail!("eval-extraction checks failed");
    }
    Ok(())
}

pub(in crate::cli) fn run_eval_gates(
    baseline_path: &str,
    thresholds_path: &str,
    golden_dataset_path: &str,
    json_out: Option<&str>,
    json: bool,
    simulate_golden_regression: bool,
    simulate_capacity_regression: bool,
) -> Result<()> {
    let report = crate::eval::gates::run_eval_gates(crate::eval::gates::EvalGateOptions {
        baseline_path: baseline_path.to_string(),
        thresholds_path: thresholds_path.to_string(),
        golden_dataset_path: golden_dataset_path.to_string(),
        simulate_golden_regression,
        simulate_capacity_regression,
    })?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = json_out {
        fs::write(path, &report_json).with_context(|| format!("write eval gate JSON {path}"))?;
    }
    if json {
        println!("{report_json}");
    } else {
        print!("{report}");
    }
    if !report.summary.passed {
        bail!("eval-gates checks failed");
    }
    Ok(())
}

pub(in crate::cli) fn run_eval_provider_comparison(args: EvalProviderComparisonArgs) -> Result<()> {
    let report = crate::eval::provider_comparison::run_provider_comparison_eval(
        crate::eval::provider_comparison::ProviderComparisonOptions {
            dataset_path: args.dataset,
            k: args.k,
            json_out: args.json_out.clone(),
            allow_api: args.allow_api,
        },
    )?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = Path::new(&args.json_out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "create provider comparison report directory {}",
                    parent.display()
                )
            })?;
        }
    }
    fs::write(&args.json_out, &report_json)
        .with_context(|| format!("write provider comparison eval JSON {}", args.json_out))?;
    if args.json {
        println!("{report_json}");
    } else {
        print!("{report}");
    }
    Ok(())
}

pub(in crate::cli) fn run_eval_graph_decision(
    dataset_path: &str,
    k: usize,
    json_out: &str,
    json: bool,
) -> Result<()> {
    let report = crate::eval::graph_decision::run_graph_decision_eval(
        crate::eval::graph_decision::GraphDecisionEvalOptions {
            dataset_path: dataset_path.to_string(),
            k,
        },
    )?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = Path::new(json_out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create graph decision eval directory {}", parent.display())
            })?;
        }
    }
    fs::write(json_out, &report_json)
        .with_context(|| format!("write graph decision eval JSON {json_out}"))?;
    if json {
        println!("{report_json}");
    } else {
        print!("{report}");
    }
    crate::eval::graph_decision::ensure_graph_decision_gate(&report)?;
    Ok(())
}

pub(in crate::cli) fn run_eval_capacity(args: EvalCapacityArgs) -> Result<()> {
    let report =
        crate::eval::capacity::run_capacity_eval(crate::eval::capacity::CapacityEvalOptions {
            dataset_path: args.dataset,
            seed: args.seed,
            scales: parse_capacity_scales(&args.scales)?,
            k: args.k,
        })?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(path) = args.json_out.as_deref() {
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("create capacity eval directory {}", parent.display())
                })?;
            }
        }
        fs::write(path, &report_json)
            .with_context(|| format!("write capacity eval JSON {path}"))?;
    }
    if args.json {
        println!("{report_json}");
    } else {
        print!("{report}");
    }
    Ok(())
}

pub(in crate::cli) fn run_eval_associative_baseline(
    dataset_path: &str,
    k: usize,
    json_out: &str,
    json: bool,
) -> Result<()> {
    let report = crate::eval::associative::run_associative_baseline(
        crate::eval::associative::AssociativeBaselineOptions {
            dataset_path: dataset_path.to_string(),
            k,
        },
    )?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = Path::new(json_out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "create associative baseline eval directory {}",
                    parent.display()
                )
            })?;
        }
    }
    fs::write(json_out, &report_json)
        .with_context(|| format!("write associative baseline eval JSON {json_out}"))?;
    if json {
        println!("{report_json}");
    } else {
        print!("{report}");
    }
    Ok(())
}

fn parse_capacity_scales(value: &str) -> Result<Vec<usize>> {
    let scales = value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<usize>()
                .with_context(|| format!("parse capacity scale {part:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    crate::eval::capacity::normalize_scales(scales)
}

pub(in crate::cli) fn run_eval_weight_grid(
    dataset_path: &str,
    k: usize,
    json_out: &str,
    json: bool,
) -> Result<()> {
    let report =
        crate::eval::weight_grid::run_weight_grid(crate::eval::weight_grid::WeightGridOptions {
            dataset_path: dataset_path.to_string(),
            k,
        })?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = Path::new(json_out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!("create weight grid eval directory {}", parent.display())
            })?;
        }
    }
    fs::write(json_out, &report_json)
        .with_context(|| format!("write weight grid eval JSON {json_out}"))?;
    if json {
        println!("{report_json}");
    } else {
        print!("{report}");
    }
    Ok(())
}

pub(in crate::cli) fn run_eval_coding_bench(args: EvalCodingBenchArgs) -> Result<()> {
    run_coding_bench_options(crate::eval::coding_bench::CodingBenchOptions {
        fixture_path: args.fixture,
        runs_per_condition: args.runs_per_condition,
        json_out: args.json_out.unwrap_or_default(),
        condition: args.condition,
        task: args.task,
        task_set: args.task_set,
        keep_workdirs: args.keep_workdirs,
        dry_run: args.dry_run,
        runner: args.runner,
        codex_bin: args.codex_bin,
        model: args.model,
        provider: args.provider,
        reasoning_effort: args.reasoning_effort,
        ignore_budget: args.ignore_budget,
    })
}

fn run_coding_bench_options(options: crate::eval::coding_bench::CodingBenchOptions) -> Result<()> {
    if options.dry_run {
        println!("{}", crate::eval::coding_bench::dry_run_plan(&options)?);
        return Ok(());
    }
    if options.json_out.trim().is_empty() {
        bail!("eval-coding-bench requires --json-out unless --dry-run is set");
    }
    let report = crate::eval::coding_bench::run_coding_bench(&options)?;
    let report_json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = Path::new(&options.json_out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "create coding benchmark report directory {}",
                    parent.display()
                )
            })?;
        }
    }
    fs::write(&options.json_out, &report_json)
        .with_context(|| format!("write coding benchmark report {}", options.json_out))?;
    println!("{report_json}");
    Ok(())
}
