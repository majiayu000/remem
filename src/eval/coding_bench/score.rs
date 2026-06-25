use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::types::{
    BenchTokenUsage, CodingBenchFailureReason, CodingMemoryAttribution,
    CodingMemoryAttributionInput, ConditionSummary, RunReport,
};

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
    let turns = runs
        .iter()
        .filter_map(|run| run.turns.map(|turns| turns as f64))
        .collect::<Vec<_>>();
    let wall = runs
        .iter()
        .map(|run| run.wall_time_ms as f64)
        .collect::<Vec<_>>();
    ConditionSummary {
        resolution_rate: resolved / n,
        tokens_total_mean: mean(&tokens),
        tokens_total_stddev: stddev(&tokens),
        turns_mean: (!turns.is_empty()).then(|| mean(&turns)),
        wall_time_ms_mean: mean(&wall),
        wall_time_ms_p95: percentile(&wall, 0.95),
        failure_counts: failure_counts(runs, false),
        memory_failure_counts: failure_counts(runs, true),
    }
}

fn failure_counts(
    runs: &[RunReport],
    memory_specific_only: bool,
) -> BTreeMap<CodingBenchFailureReason, usize> {
    let mut counts = BTreeMap::new();
    for reason in runs.iter().filter_map(|run| run.failure_reason) {
        if memory_specific_only && !reason.is_memory_specific() {
            continue;
        }
        *counts.entry(reason).or_insert(0) += 1;
    }
    counts
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

pub fn unauthorized_paths(
    changed_paths: &[String],
    allowed_paths: &[String],
    forbidden_paths: &[String],
) -> Vec<String> {
    if allowed_paths.is_empty() {
        return changed_paths
            .iter()
            .filter(|path| path_matches_any(path, forbidden_paths))
            .cloned()
            .collect();
    }
    changed_paths
        .iter()
        .filter(|path| {
            path_matches_any(path, forbidden_paths) || !path_matches_any(path, allowed_paths)
        })
        .cloned()
        .collect()
}

pub fn patch_pattern_failures(
    diff: &str,
    required_patterns: &[String],
    forbidden_patterns: &[String],
) -> Vec<String> {
    let added = added_patch_text(diff);
    let mut failures = Vec::new();
    for pattern in required_patterns {
        if !added.contains(pattern) {
            failures.push(format!("missing required patch pattern: {pattern}"));
        }
    }
    for pattern in forbidden_patterns {
        if added.contains(pattern) {
            failures.push(format!("forbidden patch pattern present: {pattern}"));
        }
    }
    failures
}

pub fn build_memory_attribution(
    input: &CodingMemoryAttributionInput,
    runner_stdout: &str,
) -> CodingMemoryAttribution {
    let injected = normalized_ids(&input.injected_memory_ids);
    let used = extract_memory_citation_ids(runner_stdout);
    let relevant = normalized_ids(&input.relevant_memory_ids);
    let forbidden = normalized_ids(&input.forbidden_memory_ids);
    let injected_set = injected.iter().copied().collect::<BTreeSet<_>>();
    let used_set = used.iter().copied().collect::<BTreeSet<_>>();
    let relevant_set = relevant.iter().copied().collect::<BTreeSet<_>>();
    let forbidden_set = forbidden.iter().copied().collect::<BTreeSet<_>>();
    let matched_used = used_set.intersection(&injected_set).count();
    let relevant_used = used_set.intersection(&relevant_set).count();
    let stale_used = used_set.intersection(&forbidden_set).count();
    let irrelevant_injected = injected_set
        .difference(&relevant_set)
        .filter(|id| !forbidden_set.contains(id))
        .count();
    let missing_relevant = relevant_set.difference(&injected_set).count();

    CodingMemoryAttribution {
        injected_memory_ids: injected,
        used_memory_ids: used,
        citation_precision: ratio_or_zero(matched_used, used_set.len()),
        citation_recall: ratio_or_one(relevant_used, relevant_set.len()),
        stale_used_count: stale_used,
        irrelevant_injection_count: irrelevant_injected,
        missing_relevant_memory_count: missing_relevant,
        memory_helped: false,
        memory_hurt: false,
    }
}

pub fn extract_memory_citation_ids(text: &str) -> Vec<i64> {
    let mut ids = BTreeSet::new();
    for line in text
        .lines()
        .filter(|line| line.contains("Memory citations:"))
    {
        let mut rest = line;
        while let Some(index) = rest.find("memory:#") {
            let after = &rest[index + "memory:#".len()..];
            let digits = after
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect::<String>();
            if let Ok(id) = digits.parse::<i64>() {
                ids.insert(id);
            }
            rest = after;
        }
    }
    ids.into_iter().collect()
}

pub fn update_memory_attribution_outcome(
    attribution: &mut CodingMemoryAttribution,
    resolved: bool,
    failure_reason: Option<CodingBenchFailureReason>,
) {
    attribution.memory_helped =
        resolved && attribution.citation_recall > 0.0 && attribution.stale_used_count == 0;
    attribution.memory_hurt = failure_reason
        .is_some_and(CodingBenchFailureReason::is_memory_specific)
        || attribution.stale_used_count > 0;
}

pub fn used_unknown_memory(attribution: &CodingMemoryAttribution) -> bool {
    let injected = attribution
        .injected_memory_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    attribution
        .used_memory_ids
        .iter()
        .any(|id| !injected.contains(id))
}

pub fn used_irrelevant_memory(
    attribution: &CodingMemoryAttribution,
    input: &CodingMemoryAttributionInput,
) -> bool {
    let relevant = input
        .relevant_memory_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let forbidden = input
        .forbidden_memory_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    attribution
        .used_memory_ids
        .iter()
        .any(|id| !relevant.contains(id) && !forbidden.contains(id))
}

fn normalized_ids(ids: &[i64]) -> Vec<i64> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn ratio_or_zero(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn ratio_or_one(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn path_matches_any(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        path == pattern
            || path
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn added_patch_text(diff: &str) -> String {
    diff.lines()
        .filter(|line| line.starts_with('+') && !line.starts_with("+++"))
        .map(|line| &line[1..])
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn parse_codex_jsonl_usage(stdout: &str) -> (BenchTokenUsage, Option<usize>) {
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
    (usage, (turns > 0).then_some(turns))
}

fn event_counts_as_turn(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|event_type| matches!(event_type, "turn.completed"))
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
    fn path_policy_checks_allowed_and_forbidden_paths() {
        let changed = vec![
            "memory_demo/slug.py".to_string(),
            "README.md".to_string(),
            "tests/test_new.py".to_string(),
        ];
        let unauthorized = unauthorized_paths(
            &changed,
            &["memory_demo".to_string(), "tests".to_string()],
            &["README.md".to_string()],
        );
        assert_eq!(unauthorized, vec!["README.md"]);
    }

    #[test]
    fn patch_patterns_scan_added_lines_only() {
        let diff = "\
diff --git a/memory_demo/slug.py b/memory_demo/slug.py
--- a/memory_demo/slug.py
+++ b/memory_demo/slug.py
@@
-    return 'legacy'
+    return 'untitled'
";
        assert!(
            patch_pattern_failures(diff, &["untitled".to_string()], &["legacy".to_string()])
                .is_empty()
        );
    }

    #[test]
    fn coding_bench_attribution_parses_memory_citations_and_recall() {
        let input = CodingMemoryAttributionInput {
            injected_memory_ids: vec![2, 1, 1],
            relevant_memory_ids: vec![1],
            forbidden_memory_ids: Vec::new(),
            gold_required_facts: vec!["fact:one".to_string()],
            gold_forbidden_facts: Vec::new(),
        };
        let attribution =
            build_memory_attribution(&input, r#"{"msg":"Memory citations: memory:#1 memory:#9"}"#);

        assert_eq!(attribution.injected_memory_ids, vec![1, 2]);
        assert_eq!(attribution.used_memory_ids, vec![1, 9]);
        assert_eq!(attribution.citation_precision, 0.5);
        assert_eq!(attribution.citation_recall, 1.0);
        assert!(used_unknown_memory(&attribution));
    }

    #[test]
    fn coding_bench_attribution_marks_stale_memory_followed() {
        let input = CodingMemoryAttributionInput {
            injected_memory_ids: vec![7],
            relevant_memory_ids: Vec::new(),
            forbidden_memory_ids: vec![7],
            gold_required_facts: Vec::new(),
            gold_forbidden_facts: vec!["fact:old_api_current".to_string()],
        };
        let mut attribution = build_memory_attribution(&input, "Memory citations: memory:#7");
        update_memory_attribution_outcome(
            &mut attribution,
            false,
            Some(CodingBenchFailureReason::StaleMemoryFollowed),
        );

        assert_eq!(attribution.stale_used_count, 1);
        assert!(attribution.memory_hurt);
        assert!(!attribution.memory_helped);
    }

    #[test]
    fn parses_nested_codex_usage() {
        let stdout = r#"{"type":"turn.completed","usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}"#;
        let (usage, turns) = parse_codex_jsonl_usage(stdout);
        assert_eq!(usage.total_tokens, 15);
        assert_eq!(turns, Some(1));
    }

    #[test]
    fn leaves_turns_unknown_without_codex_turn_events() {
        let stdout = r#"{"type":"item.completed","item":{"type":"agent_message","text":"OK"}}"#;
        let (usage, turns) = parse_codex_jsonl_usage(stdout);
        assert_eq!(usage.total_tokens, 0);
        assert_eq!(turns, None);
    }
}
