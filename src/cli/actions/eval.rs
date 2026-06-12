use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::db;

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
) -> Result<()> {
    let report = crate::eval::gates::run_eval_gates(crate::eval::gates::EvalGateOptions {
        baseline_path: baseline_path.to_string(),
        thresholds_path: thresholds_path.to_string(),
        golden_dataset_path: golden_dataset_path.to_string(),
        simulate_golden_regression,
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
