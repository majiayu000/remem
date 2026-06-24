use serde_json::Value;

use super::types::{BenchTokenUsage, ConditionSummary, RunReport};

pub fn summarize_runs(runs: &[RunReport]) -> ConditionSummary {
    if runs.is_empty() {
        return ConditionSummary::default();
    }
    let n = runs.len() as f64;
    let resolved = runs.iter().filter(|run| run.resolved).count() as f64;
    let tokens = runs
        .iter()
        .map(|run| run.usage.total_tokens as f64)
        .collect::<Vec<_>>();
    let turns = runs.iter().map(|run| run.turns as f64).collect::<Vec<_>>();
    let wall = runs
        .iter()
        .map(|run| run.wall_time_ms as f64)
        .collect::<Vec<_>>();
    ConditionSummary {
        resolution_rate: resolved / n,
        tokens_total_mean: mean(&tokens),
        tokens_total_stddev: stddev(&tokens),
        turns_mean: mean(&turns),
        wall_time_ms_mean: mean(&wall),
        wall_time_ms_p95: percentile(&wall, 0.95),
    }
}

pub fn parse_changed_paths(porcelain: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in porcelain.lines() {
        if line.len() < 4 {
            continue;
        }
        let path = if line.starts_with("R ") || line.starts_with("RM") || line.starts_with("R  ") {
            line.split(" -> ").last().unwrap_or(&line[3..])
        } else {
            &line[3..]
        };
        let path = path.trim();
        if !path.is_empty() && !is_generated_path(path) {
            paths.push(path.to_string());
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn is_generated_path(path: &str) -> bool {
    path.contains("__pycache__/")
        || path.ends_with(".pyc")
        || path == ".pytest_cache/"
        || path.starts_with(".pytest_cache/")
}

pub fn unauthorized_paths(changed_paths: &[String], allowed_paths: &[String]) -> Vec<String> {
    if allowed_paths.is_empty() {
        return Vec::new();
    }
    changed_paths
        .iter()
        .filter(|path| {
            !allowed_paths.iter().any(|allowed| {
                path == &allowed
                    || path
                        .strip_prefix(allowed)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        })
        .cloned()
        .collect()
}

pub fn parse_codex_jsonl_usage(stdout: &str) -> (BenchTokenUsage, usize) {
    let mut usage = BenchTokenUsage::default();
    let mut turns = 0;
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if event_counts_as_turn(&value) {
            turns += 1;
        }
        for candidate in usage_candidates(&value) {
            if candidate.total_tokens >= usage.total_tokens {
                usage = candidate;
            }
        }
    }
    (usage, turns)
}

fn event_counts_as_turn(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|event_type| {
            matches!(
                event_type,
                "agent_message" | "assistant_message" | "message"
            )
        })
}

fn usage_candidates(value: &Value) -> Vec<BenchTokenUsage> {
    let mut out = Vec::new();
    collect_usage_candidates(value, &mut out);
    out
}

fn collect_usage_candidates(value: &Value, out: &mut Vec<BenchTokenUsage>) {
    match value {
        Value::Object(map) => {
            let input = number_field(map.get("input_tokens").or_else(|| map.get("prompt_tokens")));
            let output = number_field(
                map.get("output_tokens")
                    .or_else(|| map.get("completion_tokens")),
            );
            let total = number_field(map.get("total_tokens"));
            if input > 0 || output > 0 || total > 0 {
                out.push(BenchTokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                    total_tokens: total.max(input.saturating_add(output)),
                });
            }
            for value in map.values() {
                collect_usage_candidates(value, out);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_usage_candidates(value, out);
            }
        }
        _ => {}
    }
}

fn number_field(value: Option<&Value>) -> u64 {
    value.and_then(Value::as_u64).unwrap_or(0)
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn stddev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = mean(values);
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((sorted.len() as f64 - 1.0) * p).ceil() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_paths_without_losing_first_character() {
        let paths = parse_changed_paths(
            " M memory_demo/slug.py\n?? tests/test_new.py\n?? memory_demo/__pycache__/\n",
        );
        assert_eq!(paths, vec!["memory_demo/slug.py", "tests/test_new.py"]);
    }

    #[test]
    fn parses_nested_codex_usage() {
        let stdout = r#"{"type":"agent_message","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}"#;
        let (usage, turns) = parse_codex_jsonl_usage(stdout);
        assert_eq!(usage.total_tokens, 15);
        assert_eq!(turns, 1);
    }
}
