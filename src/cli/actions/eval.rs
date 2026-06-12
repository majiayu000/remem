use std::fs;

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
