use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{
    MeasurementState, OutcomeScorecard, PublicEvidence, ScorecardComponent, ScorecardField,
};

pub(super) fn build_scorecard(
    public: &PublicEvidence,
    security_report_path: &Path,
) -> OutcomeScorecard {
    let coding = public
        .report
        .as_ref()
        .filter(|report| report.artifact_verifier.passed)
        .map(|report| &report.coding_task_outcomes);
    let task_completion_source = coding.and_then(|runs| {
        artifact_sources(
            runs.iter()
                .filter(|run| run.target_started == Some(true))
                .map(|run| run.report_path.as_str())
                .collect(),
        )
    });
    let memory_help_source = coding.and_then(|runs| {
        artifact_sources(
            runs.iter()
                .filter(|run| run.memory_helped.is_some())
                .map(|run| run.report_path.as_str())
                .collect(),
        )
    });
    let task_completion = coding.map(|runs| {
        task_completion_counts(runs.iter().map(|run| (run.target_started, run.resolved)))
    });
    let security_source = exact_artifact_source(security_report_path);
    let fields = vec![
        ratio_field(
            "task_completion_rate",
            task_completion.map(|(resolved, _)| resolved),
            task_completion.map(|(_, started)| started),
            "verified committed coding outcomes; current artifacts may be smoke-only",
            "resolved coding runs",
            "all eligible coding runs",
            "registered GH931 threshold when official matrix is complete",
            "smoke_or_official_as_declared_by_source",
            task_completion_source,
        ),
        ratio_field(
            "correct_memory_help_rate",
            coding.map(|runs| {
                runs.iter()
                    .filter(|run| run.memory_helped == Some(true))
                    .count() as f64
            }),
            coding.map(|runs| {
                runs.iter()
                    .filter(|run| run.memory_helped.is_some())
                    .count() as f64
            }),
            "verified coding runs with an explicit memory_helped verdict",
            "runs where memory_helped is true",
            "runs with a measured memory_helped verdict",
            "reported; no public threshold until GH931 official matrix",
            "smoke_or_official_as_declared_by_source",
            memory_help_source,
        ),
        unavailable_field(
            "repeated_explanation_rate",
            "coding runs with an instrumented repeated-explanation opportunity",
            "repeated explanation events",
            "eligible explanation opportunities",
            "no checked-in artifact records this measure",
        ),
        unavailable_field(
            "wrong_memory_injection_rate",
            "context emissions with a deterministic wrong-memory oracle",
            "wrong-memory injections",
            "eligible context emissions",
            "current artifacts do not carry the required injection oracle",
        ),
        unavailable_field(
            "stale_memory_injection_rate",
            "context emissions with a deterministic staleness oracle",
            "stale-memory injections",
            "eligible context emissions",
            "stale-followed evidence is not an injection-rate denominator",
        ),
        unavailable_field(
            "cross_scope_injection_rate",
            "context emissions with sealed project/owner scope truth",
            "cross-scope injections",
            "eligible context emissions",
            "GH935 live artifacts are unavailable",
        ),
        security_ratio_field(public, security_source.clone()),
        abstention_field(public, security_source),
        latency_field(public, security_report_path),
        unavailable_field(
            "maintenance_time_and_ai_usage",
            "official GH931 target-blind curator and remem_e2e runs",
            "maintenance minutes and AI usage units",
            "100 eligible sessions and official task runs",
            "governed official execution has not occurred",
        ),
    ];
    OutcomeScorecard {
        schema_version: 1,
        measurement_states: [
            MeasurementState::Measured,
            MeasurementState::Unavailable,
            MeasurementState::NotApplicable,
        ],
        fields,
    }
}

pub(super) fn task_completion_counts(
    runs: impl Iterator<Item = (Option<bool>, bool)>,
) -> (f64, f64) {
    runs.filter(|(target_started, _)| *target_started == Some(true))
        .fold((0.0, 0.0), |(resolved, started), (_, did_resolve)| {
            (resolved + f64::from(did_resolve), started + 1.0)
        })
}

pub(super) fn ratio_field(
    id: &'static str,
    numerator: Option<f64>,
    denominator: Option<f64>,
    population: &str,
    numerator_definition: &str,
    denominator_definition: &str,
    threshold: &str,
    claim_level: &str,
    source: Option<String>,
) -> ScorecardField {
    let measured = numerator.is_some() && denominator.is_some_and(|value| value > 0.0);
    let (numerator, denominator) = if measured {
        (numerator, denominator)
    } else {
        (None, None)
    };
    let mut values = BTreeMap::new();
    if let (Some(numerator), Some(denominator)) = (numerator, denominator) {
        if denominator > 0.0 {
            values.insert("rate".to_string(), numerator / denominator);
        }
    }
    ScorecardField {
        id,
        measurement_state: if measured {
            MeasurementState::Measured
        } else {
            MeasurementState::Unavailable
        },
        eligible_population: population.to_string(),
        numerator: ScorecardComponent {
            definition: numerator_definition.to_string(),
            value: numerator,
        },
        denominator: ScorecardComponent {
            definition: denominator_definition.to_string(),
            value: denominator,
        },
        values,
        threshold: threshold.to_string(),
        source: measured.then_some(source).flatten(),
        claim_level: claim_level.to_string(),
        note: if measured {
            "Measured only for the population and claim level declared here.".to_string()
        } else {
            "Unavailable fields do not pass any gate.".to_string()
        },
    }
}

pub(super) fn unavailable_field(
    id: &'static str,
    population: &str,
    numerator: &str,
    denominator: &str,
    note: &str,
) -> ScorecardField {
    ScorecardField {
        id,
        measurement_state: MeasurementState::Unavailable,
        eligible_population: population.to_string(),
        numerator: ScorecardComponent {
            definition: numerator.to_string(),
            value: None,
        },
        denominator: ScorecardComponent {
            definition: denominator.to_string(),
            value: None,
        },
        values: BTreeMap::new(),
        threshold: "unavailable_cannot_pass".to_string(),
        source: None,
        claim_level: "no_claim".to_string(),
        note: note.to_string(),
    }
}

fn security_ratio_field(public: &PublicEvidence, source: Option<String>) -> ScorecardField {
    let denominator = security_number(public, "/aggregate_metrics/policy/non_retention_cases");
    let rate = security_number(public, "/aggregate_metrics/policy/non_retention_leak_rate");
    ratio_field(
        "poison_policy_leak_rate",
        denominator.zip(rate).map(|(count, value)| count * value),
        denominator,
        "production-path adversarial cases governed by non-retention policy",
        "persisted policy leaks",
        "eligible non-retention cases",
        "rate == 0",
        "directional_memory_suite_no_public_claim",
        source,
    )
}

fn abstention_field(public: &PublicEvidence, source: Option<String>) -> ScorecardField {
    ratio_field(
        "abstention_rate",
        security_number(
            public,
            "/aggregate_metrics/failure_decomposition/overall/policy_abstention",
        ),
        security_number(public, "/aggregate_metrics/run_count"),
        "production-path adversarial-policy v2 runs",
        "runs ending in policy abstention",
        "verified adversarial runs",
        "reported with accuracy; rate alone is not a pass criterion",
        "directional_memory_suite_no_public_claim",
        source,
    )
}

fn latency_field(_public: &PublicEvidence, _security_report_path: &Path) -> ScorecardField {
    ScorecardField {
        id: "foreground_latency_p50_p95",
        measurement_state: MeasurementState::Unavailable,
        eligible_population: "production-path adversarial-policy v2 remem_default runs".to_string(),
        numerator: ScorecardComponent {
            definition: "not applicable: percentile statistic".to_string(),
            value: None,
        },
        denominator: ScorecardComponent {
            definition: "timed verified runs".to_string(),
            value: None,
        },
        values: BTreeMap::new(),
        threshold: "unavailable_cannot_pass".to_string(),
        source: None,
        claim_level: "no_claim".to_string(),
        note: "Current benchmark latency values are synthetic fixture estimates, not wall-clock measurements."
            .to_string(),
    }
}

fn exact_artifact_source(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(format!(
        "{}#sha256={:x}",
        path.to_string_lossy(),
        Sha256::digest(bytes)
    ))
}

fn artifact_sources(paths: std::collections::BTreeSet<&str>) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    paths
        .into_iter()
        .map(|relative| {
            exact_artifact_source(
                Path::new(super::DEFAULT_PUBLIC_ROOT)
                    .join(relative)
                    .as_path(),
            )
        })
        .collect::<Option<Vec<_>>>()
        .map(|sources| sources.join(","))
}

fn security_number(public: &PublicEvidence, pointer: &str) -> Option<f64> {
    if !public.security_authority.passed {
        return None;
    }
    public.security.as_ref()?.pointer(pointer)?.as_f64()
}
