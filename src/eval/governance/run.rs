use std::fmt::{self, Display};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection};

use super::fixture::{
    forbidden_hits, memory_owner_checks, seed_fixture, summary_candidate_owner_checks, CORPUS_NAME,
    EXPECTED_SUMMARY_CANDIDATES, NESTED_CRATE_PROJECT, NESTED_SRC_PROJECT, PROJECT,
    SEARCH_SCENARIOS,
};
use super::types::{
    CandidateSummary, ContextReport, GovernanceEvalMetadata, GovernanceEvalOptions,
    GovernanceEvalReport, GovernanceMetricSummary, OwnerCheckReport, QueryReport, RateMetric,
};

pub fn run_sandbox_eval(options: GovernanceEvalOptions) -> Result<GovernanceEvalReport> {
    let temp_data_dir = TempDataDir::new()?;
    let data_dir = temp_data_dir.path.clone();
    crate::db::core::with_data_dir(&data_dir, || {
        crate::log::with_log_dir(&data_dir, || {
            let result = run_sandbox_eval_inner(options, &data_dir);
            temp_data_dir.cleanup_result(result)
        })
    })
}

fn run_sandbox_eval_inner(
    options: GovernanceEvalOptions,
    data_dir: &Path,
) -> Result<GovernanceEvalReport> {
    let k = options.k.max(1);
    ensure!(
        crate::db::db_path().starts_with(data_dir),
        "governance eval data dir override failed"
    );
    let mut conn = Connection::open_in_memory().context("open in-memory governance eval DB")?;
    crate::migrate::run_migrations(&conn).context("migrate in-memory governance eval DB")?;

    let fixture = seed_fixture(&mut conn)?;
    let mut owner_checks = memory_owner_checks(&conn, &fixture.expected_owners)?;
    owner_checks.extend(summary_candidate_owner_checks(
        &conn,
        &fixture.expected_candidate_owners,
    )?);
    let queries = evaluate_queries(&conn, k)?;
    let context = evaluate_context(&conn)?;
    let lifecycle_counts = fixture.lifecycle_counts;
    let summary_candidates = load_candidate_summary(&conn)?;

    let owner_routing_accuracy = RateMetric::new(
        owner_checks.iter().filter(|check| check.pass).count(),
        owner_checks.len(),
    );
    let evidence_recall_at_k = evidence_recall(&queries);
    let active_current_precision = active_current_precision(&queries);
    let stale_exclusion_rate = category_pass_rate(&queries, "stale_exclusion");
    let context_injection_precision = context_injection_precision(&context);

    let mut failing_examples = collect_failures(&owner_checks, &queries, &context);
    if summary_candidates.total != EXPECTED_SUMMARY_CANDIDATES {
        failing_examples.push(format!(
            "summary candidates expected total={} got total={}",
            EXPECTED_SUMMARY_CANDIDATES, summary_candidates.total
        ));
    }
    if summary_candidates.active_summary_memories != 0
        || summary_candidates.pending_review != summary_candidates.total
    {
        failing_examples.push(format!(
            "summary candidates should stay pending: total={} pending={} auto_promoted={} active_memories={}",
            summary_candidates.total,
            summary_candidates.pending_review,
            summary_candidates.auto_promoted,
            summary_candidates.active_summary_memories
        ));
    }
    let expected_lifecycle_counts = crate::eval::governance::LifecycleCounts {
        add: 1,
        update: 1,
        invalidate: 1,
        noop: 1,
        defer: 1,
    };
    if lifecycle_counts != expected_lifecycle_counts {
        failing_examples.push(format!(
            "lifecycle counts expected {:?}, got {:?}",
            expected_lifecycle_counts, lifecycle_counts
        ));
    }

    let all_checks_passed = owner_routing_accuracy.is_perfect()
        && evidence_recall_at_k.is_perfect()
        && active_current_precision.is_perfect()
        && stale_exclusion_rate.is_perfect()
        && context.pass
        && summary_candidates.total == EXPECTED_SUMMARY_CANDIDATES
        && summary_candidates.pending_review == summary_candidates.total
        && summary_candidates.active_summary_memories == 0
        && failing_examples.is_empty();

    Ok(GovernanceEvalReport {
        metadata: GovernanceEvalMetadata {
            corpus: CORPUS_NAME.to_string(),
            storage: "in-memory sqlite".to_string(),
            data_dir: data_dir.display().to_string(),
            real_db_touched: false,
            project: PROJECT.to_string(),
            nested_projects: vec![
                NESTED_SRC_PROJECT.to_string(),
                NESTED_CRATE_PROJECT.to_string(),
            ],
            k,
        },
        metrics: GovernanceMetricSummary {
            owner_routing_accuracy,
            evidence_recall_at_k,
            active_current_precision,
            stale_exclusion_rate,
            context_injection_precision,
            all_checks_passed,
        },
        lifecycle_counts,
        summary_candidates,
        owner_checks,
        queries,
        context,
        failing_examples,
    })
}

fn unique_temp_data_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "remem-governance-eval-{}-{}",
        std::process::id(),
        nanos
    ))
}

struct TempDataDir {
    path: PathBuf,
    cleaned: bool,
}

impl TempDataDir {
    fn new() -> Result<Self> {
        let path = unique_temp_data_dir();
        std::fs::create_dir_all(&path)
            .with_context(|| format!("create governance eval data dir {}", path.display()))?;
        Ok(Self {
            path,
            cleaned: false,
        })
    }

    fn cleanup_result<T>(mut self, result: Result<T>) -> Result<T> {
        let cleanup = self.cleanup();
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Ok(_), Err(err)) => Err(err),
            (Err(err), Ok(())) => Err(err),
            (Err(err), Err(cleanup_err)) => {
                crate::log::warn(
                    "eval-governance",
                    &format!("cleanup failed after eval error: {}", cleanup_err),
                );
                Err(err)
            }
        }
    }

    fn cleanup(&mut self) -> Result<()> {
        std::fs::remove_dir_all(&self.path)
            .with_context(|| format!("remove governance eval data dir {}", self.path.display()))?;
        self.cleaned = true;
        Ok(())
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        if self.cleaned {
            return;
        }
        if let Err(cleanup_err) = std::fs::remove_dir_all(&self.path) {
            crate::log::warn(
                "eval-governance",
                &format!("cleanup failed during drop: {}", cleanup_err),
            );
        }
    }
}

fn evaluate_queries(conn: &Connection, k: usize) -> Result<Vec<QueryReport>> {
    SEARCH_SCENARIOS
        .iter()
        .map(|scenario| evaluate_query(conn, scenario, k))
        .collect()
}

fn evaluate_query(
    conn: &Connection,
    scenario: &super::fixture::SearchScenario,
    k: usize,
) -> Result<QueryReport> {
    let results = crate::retrieval::search::search_with_branch(
        conn,
        Some(scenario.query),
        Some(scenario.project),
        scenario.memory_type,
        k.max(1) as i64,
        0,
        false,
        scenario.branch,
    )?;
    let result_topic_keys = results
        .iter()
        .filter_map(|memory| memory.topic_key.clone())
        .collect::<Vec<_>>();
    let result_titles = results
        .iter()
        .map(|memory| memory.title.clone())
        .collect::<Vec<_>>();
    let matched_expected = scenario
        .expected_topic_keys
        .iter()
        .filter(|expected| result_topic_keys.iter().any(|actual| actual == *expected))
        .count();
    let forbidden_hits = forbidden_hits(&results, scenario.forbidden_title_contains);
    let unexpected_hits = results
        .iter()
        .filter(|memory| {
            memory
                .topic_key
                .as_deref()
                .is_none_or(|topic_key| !scenario.allowed_topic_keys.contains(&topic_key))
        })
        .map(|memory| {
            format!(
                "{} ({})",
                memory.topic_key.as_deref().unwrap_or("<missing-topic-key>"),
                memory.title
            )
        })
        .collect::<Vec<_>>();
    let pass = matched_expected == scenario.expected_topic_keys.len()
        && forbidden_hits.is_empty()
        && unexpected_hits.is_empty();

    Ok(QueryReport {
        id: scenario.id.to_string(),
        category: scenario.category.to_string(),
        query: scenario.query.to_string(),
        project: scenario.project.to_string(),
        memory_type: scenario.memory_type.map(str::to_string),
        branch: scenario.branch.map(str::to_string),
        expected_topic_keys: scenario
            .expected_topic_keys
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        result_topic_keys,
        result_titles,
        matched_expected,
        forbidden_hits,
        unexpected_hits,
        pass,
    })
}

fn evaluate_context(conn: &Connection) -> Result<ContextReport> {
    let snapshot = crate::context::governance_eval_snapshot(conn, PROJECT, Some("main"))?;
    let expected_topic_keys = vec![
        "repo-dnd-pointer".to_string(),
        "repo-agent-crate-cache".to_string(),
        "repo-codex-file-exception".to_string(),
        "repo-toolbar-color".to_string(),
        "repo-test-command".to_string(),
        "decision-1111111111111111".to_string(),
    ];
    let forbidden_titles = snapshot
        .rendered_output
        .lines()
        .filter(|title| {
            [
                "Codex approval mode",
                "Grok API image references",
                "Warp terminal launch config",
                "Toolbar color was blue",
                "Feature branch wasm snapshot",
            ]
            .iter()
            .any(|needle| title.contains(needle))
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    let expected_present = expected_topic_keys.iter().all(|expected| {
        snapshot
            .memory_topic_keys
            .iter()
            .any(|actual| actual == expected)
    });
    let unexpected_topic_keys = snapshot
        .memory_topic_keys
        .iter()
        .filter(|topic_key| !expected_topic_keys.contains(topic_key))
        .cloned()
        .collect::<Vec<_>>();
    let pass = expected_present
        && unexpected_topic_keys.is_empty()
        && forbidden_titles.is_empty()
        && snapshot.unsafe_owner_included == 0;

    Ok(ContextReport {
        expected_topic_keys,
        included_topic_keys: snapshot.memory_topic_keys,
        included_titles: snapshot.memory_titles,
        forbidden_titles,
        unexpected_topic_keys,
        unsafe_owner_included: snapshot.unsafe_owner_included,
        excluded_owner_titles: snapshot.excluded_owner_titles,
        pass,
    })
}

fn load_candidate_summary(conn: &Connection) -> Result<CandidateSummary> {
    let total = candidate_count(conn, None)?;
    let pending_review = candidate_count(conn, Some("pending_review"))?;
    let auto_promoted = candidate_count(conn, Some("auto_promoted"))?;
    let active_summary_memories: usize = conn.query_row(
        "SELECT COUNT(*)
         FROM memories
         WHERE source_candidate_id IN (SELECT id FROM memory_candidates)
           AND status = 'active'",
        [],
        |row| row.get::<_, i64>(0),
    )? as usize;
    Ok(CandidateSummary {
        total,
        pending_review,
        auto_promoted,
        active_summary_memories,
    })
}

fn candidate_count(conn: &Connection, status: Option<&str>) -> Result<usize> {
    match status {
        Some(status) => Ok(conn.query_row(
            "SELECT COUNT(*) FROM memory_candidates WHERE review_status = ?1",
            params![status],
            |row| row.get::<_, i64>(0),
        )? as usize),
        None => Ok(
            conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
                row.get::<_, i64>(0)
            })? as usize,
        ),
    }
}

fn evidence_recall(queries: &[QueryReport]) -> RateMetric {
    let total = queries
        .iter()
        .map(|query| query.expected_topic_keys.len())
        .sum::<usize>();
    let passed = queries
        .iter()
        .map(|query| query.matched_expected)
        .sum::<usize>();
    RateMetric::new(passed, total)
}

fn category_pass_rate(queries: &[QueryReport], category: &str) -> RateMetric {
    let selected = queries
        .iter()
        .filter(|query| query.category == category)
        .collect::<Vec<_>>();
    RateMetric::new(
        selected.iter().filter(|query| query.pass).count(),
        selected.len(),
    )
}

fn active_current_precision(queries: &[QueryReport]) -> RateMetric {
    let selected = queries
        .iter()
        .filter(|query| query.category == "active_current");
    let mut relevant_results = 0usize;
    let mut total_results = 0usize;
    for query in selected {
        relevant_results += query
            .result_topic_keys
            .iter()
            .filter(|topic_key| query.expected_topic_keys.contains(*topic_key))
            .count();
        total_results += query.result_topic_keys.len();
    }
    RateMetric::new(relevant_results, total_results)
}

fn context_injection_precision(context: &ContextReport) -> RateMetric {
    let expected_included = context
        .included_topic_keys
        .iter()
        .filter(|topic_key| context.expected_topic_keys.contains(topic_key))
        .count();
    RateMetric::new(expected_included, context.included_topic_keys.len())
}

fn collect_failures(
    owner_checks: &[OwnerCheckReport],
    queries: &[QueryReport],
    context: &ContextReport,
) -> Vec<String> {
    let mut failures = Vec::new();
    for check in owner_checks.iter().filter(|check| !check.pass) {
        failures.push(format!(
            "{} owner expected {}:{} target={:?}, got {:?}:{:?} target={:?}",
            check.object_ref,
            check.expected_scope,
            check.expected_key,
            check.expected_target_project,
            check.actual_scope,
            check.actual_key,
            check.actual_target_project
        ));
    }
    for query in queries.iter().filter(|query| !query.pass) {
        failures.push(format!(
            "{} expected {:?}, forbidden {:?}, got topics {:?}, titles {:?}",
            query.id,
            query.expected_topic_keys,
            query.forbidden_hits,
            query.result_topic_keys,
            query.result_titles
        ));
    }
    for query in queries
        .iter()
        .filter(|query| !query.unexpected_hits.is_empty())
    {
        failures.push(format!(
            "{} had unexpected hits {:?}",
            query.id, query.unexpected_hits
        ));
    }
    if !context.pass {
        failures.push(format!(
            "context expected {:?}, included {:?}, unexpected {:?}, forbidden {:?}, unsafe_owner_included={}",
            context.expected_topic_keys,
            context.included_topic_keys,
            context.unexpected_topic_keys,
            context.forbidden_titles,
            context.unsafe_owner_included
        ));
    }
    failures
}

impl Display for GovernanceEvalReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "=== remem eval-governance ({}, k={}) ===",
            self.metadata.corpus, self.metadata.k
        )?;
        writeln!(
            f,
            "storage: {}; real_db_touched={}",
            self.metadata.storage, self.metadata.real_db_touched
        )?;
        write_metric(
            f,
            "owner_routing_accuracy",
            &self.metrics.owner_routing_accuracy,
        )?;
        write_metric(
            f,
            "evidence_recall_at_k",
            &self.metrics.evidence_recall_at_k,
        )?;
        write_metric(
            f,
            "active_current_precision",
            &self.metrics.active_current_precision,
        )?;
        write_metric(
            f,
            "stale_exclusion_rate",
            &self.metrics.stale_exclusion_rate,
        )?;
        write_metric(
            f,
            "context_injection_precision",
            &self.metrics.context_injection_precision,
        )?;
        writeln!(
            f,
            "lifecycle: add={} update={} invalidate={} noop={} defer={}",
            self.lifecycle_counts.add,
            self.lifecycle_counts.update,
            self.lifecycle_counts.invalidate,
            self.lifecycle_counts.noop,
            self.lifecycle_counts.defer
        )?;
        writeln!(
            f,
            "summary_candidates: total={} pending_review={} auto_promoted={} active_summary_memories={}",
            self.summary_candidates.total,
            self.summary_candidates.pending_review,
            self.summary_candidates.auto_promoted,
            self.summary_candidates.active_summary_memories
        )?;
        writeln!(f, "all_checks_passed: {}", self.metrics.all_checks_passed)?;
        if self.failing_examples.is_empty() {
            writeln!(f, "failures: none")?;
        } else {
            writeln!(f, "failures:")?;
            for failure in &self.failing_examples {
                writeln!(f, "- {failure}")?;
            }
        }
        Ok(())
    }
}

fn write_metric(f: &mut fmt::Formatter<'_>, label: &str, metric: &RateMetric) -> fmt::Result {
    writeln!(
        f,
        "{}: {:.1}% ({}/{})",
        label,
        metric.rate * 100.0,
        metric.passed,
        metric.total
    )
}
