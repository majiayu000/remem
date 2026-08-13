use std::fs;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use serde::Serialize;

use super::run_plan::build_run_plan;
use super::types::{BenchCondition, CodingBenchOptions, CodingBenchTask};

pub(super) fn write_dry_run_json(
    options: &CodingBenchOptions,
    conditions: &[BenchCondition],
    tasks: &[&CodingBenchTask],
    planned_runs: usize,
) -> Result<()> {
    let canonical_plan = build_run_plan(conditions, tasks.len(), options.runs_per_condition);
    ensure!(
        canonical_plan.len() == planned_runs,
        "coding benchmark dry-run total drifted from canonical run plan: expected {planned_runs}, got {}",
        canonical_plan.len()
    );
    let report = DryRunReport {
        schema_version: 1,
        fixture_path: options.fixture_path.clone(),
        matrix: effective_matrix(options).to_string(),
        task_set: options.task_set.clone(),
        runs_per_condition: options.runs_per_condition,
        planned_runs,
        conditions: conditions
            .iter()
            .map(|condition| condition.as_str().to_string())
            .collect(),
        task_ids: tasks.iter().map(|task| task.id.clone()).collect(),
        planned_tuples: canonical_plan
            .into_iter()
            .map(|entry| DryRunTuple {
                condition: entry.condition.as_str().to_string(),
                task_id: tasks[entry.task_index].id.clone(),
                run_index: entry.run_index,
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = Path::new(&options.json_out).parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "create coding benchmark dry-run report directory {}",
                    parent.display()
                )
            })?;
        }
    }
    fs::write(&options.json_out, json)
        .with_context(|| format!("write coding benchmark dry-run report {}", options.json_out))
}

pub(super) fn effective_matrix(options: &CodingBenchOptions) -> &str {
    if options.condition.is_some() {
        "condition"
    } else if options.matrix.trim().is_empty() {
        "primary"
    } else {
        options.matrix.trim()
    }
}

#[derive(Debug, Serialize)]
struct DryRunReport {
    schema_version: u32,
    fixture_path: String,
    matrix: String,
    task_set: String,
    runs_per_condition: usize,
    planned_runs: usize,
    conditions: Vec<String>,
    task_ids: Vec<String>,
    planned_tuples: Vec<DryRunTuple>,
}

#[derive(Debug, Serialize)]
struct DryRunTuple {
    condition: String,
    task_id: String,
    run_index: usize,
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::eval::coding_bench::fixture::{load_fixture, selected_conditions, selected_tasks};
    use crate::eval::coding_bench::runner::dry_run_plan;

    #[test]
    fn builds_expanded_default_matrix() -> Result<()> {
        let fixture = load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let options = options_with_json_out("/tmp/remem-coding-bench.json");
        let conditions = selected_conditions(&options)?;
        let tasks = selected_tasks(&fixture, &options)?;
        assert_eq!(conditions.len(), 3);
        assert_eq!(
            conditions,
            vec![
                BenchCondition::NoMemory,
                BenchCondition::CuratedFileBudgeted,
                BenchCondition::RememE2e,
            ]
        );
        assert_eq!(tasks.len(), 16);
        assert_eq!(
            conditions.len() * tasks.len() * options.runs_per_condition,
            144
        );
        Ok(())
    }

    #[test]
    fn dry_run_writes_gh931_primary_plan_json_with_current_ids() -> Result<()> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let json_out = std::env::temp_dir().join(format!(
            "remem-gh931-plan-{}-{unique}.json",
            std::process::id()
        ));
        let options = options_with_json_out(&json_out.to_string_lossy());

        let text = dry_run_plan(&options)?;
        assert!(text.contains("planned_runs: 144"));
        assert!(text.contains("- remem_e2e "));
        assert!(!text.contains("- remem "));
        assert!(!text.contains("- curated_file "));

        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json_out)?)?;
        assert_eq!(json["planned_runs"], 144);
        assert_eq!(
            json["conditions"],
            serde_json::json!(["no_memory", "curated_file_budgeted", "remem_e2e"])
        );
        assert_eq!(json["planned_tuples"].as_array().map(Vec::len), Some(144));
        let tuples = json["planned_tuples"].as_array().expect("planned tuples");
        let first_task = &json["task_ids"][0];
        for condition in ["no_memory", "curated_file_budgeted", "remem_e2e"] {
            let run_indices = tuples
                .iter()
                .filter(|tuple| tuple["condition"] == condition && tuple["task_id"] == *first_task)
                .map(|tuple| tuple["run_index"].as_u64().expect("numeric run index"))
                .collect::<Vec<_>>();
            assert_eq!(run_indices, vec![0, 1, 2]);
        }
        assert!(tuples.iter().all(|tuple| tuple["run_index"] != 3));
        let _ = std::fs::remove_file(&json_out);
        Ok(())
    }

    #[test]
    fn legacy_bare_condition_ids_do_not_parse() {
        assert_eq!(BenchCondition::parse("remem"), None);
        assert_eq!(BenchCondition::parse("curated_file"), None);
        assert_eq!(BenchCondition::parse("remem_preloaded"), None);
        assert_eq!(
            BenchCondition::parse("remem_seeded_sessionstart"),
            Some(BenchCondition::RememSeededSessionStart)
        );
        assert_eq!(
            BenchCondition::parse("curated_file_expert"),
            Some(BenchCondition::CuratedFileExpert)
        );
    }

    fn options_with_json_out(json_out: &str) -> CodingBenchOptions {
        CodingBenchOptions {
            fixture_path: "eval/coding-bench/fixtures/tasks.json".to_string(),
            runs_per_condition: 3,
            json_out: json_out.to_string(),
            condition: None,
            matrix: "primary".to_string(),
            task: None,
            task_set: "full".to_string(),
            keep_workdirs: false,
            dry_run: true,
            runner: "noop".to_string(),
            codex_bin: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            provider: None,
            reasoning_effort: "medium".to_string(),
            ignore_budget: false,
        }
    }
}
