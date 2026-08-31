use std::collections::BTreeMap;
use std::path::Path;

use super::{
    MeasurementState, OutcomeScorecard, PublicEvidence, ScorecardComponent, ScorecardField,
};
use crate::eval::bench_artifact::AuthorityStatus;

pub(super) fn build_scorecard(
    public: &PublicEvidence,
    security_report_path: &Path,
) -> OutcomeScorecard {
    let gh931 = public.authority_verdict().map(|verdict| &verdict.gh931);
    let task_completion = gh931
        .filter(|authority| {
            authority.completeness.complete
                && authority.completeness.attempts_ready
                && authority.completeness.machine_outcomes_ready
        })
        .and_then(|authority| {
            authority
                .condition_completion
                .iter()
                .find(|completion| completion.condition == "remem_e2e")
        });
    let task_completion_source = gh931.and_then(|authority| {
        authority
            .report
            .as_ref()
            .map(|report| format!("{}#sha256={}", report.path, report.sha256))
    });
    let security_source = public.security_source.clone();
    let fields = vec![
        ratio_field(
            "task_completion_rate",
            task_completion.map(|completion| completion.resolved as f64),
            task_completion.map(|completion| completion.eligible_started as f64),
            "issue385-v1/official-v1 remem_e2e runs with target_started=true",
            "resolved eligible remem_e2e runs",
            "eligible started remem_e2e runs",
            "registered GH931 threshold when official matrix is complete",
            gh931_claim_level(gh931),
            task_completion_source.clone(),
        ),
        unavailable_field(
            "correct_memory_help_rate",
            "official remem_e2e runs with verifier-modeled memory-help authority",
            "runs where memory_helped is true",
            "runs with a measured memory_helped verdict",
            "the runtime verdict does not yet expose an authoritative memory-help population",
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
        maintenance_field(gh931, task_completion_source),
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

fn maintenance_field(
    authority: Option<&crate::eval::bench_artifact::Gh931AuthorityVerdict>,
    source: Option<String>,
) -> ScorecardField {
    let measured = authority.and_then(|authority| {
        let maintenance = &authority.maintenance;
        if maintenance.status != AuthorityStatus::Pass || maintenance.curator_sessions == 0 {
            return None;
        }
        Some((
            authority,
            maintenance.curator_minutes?,
            maintenance.curated_minutes_per_100_sessions?,
            maintenance
                .remem_sessions
                .filter(|sessions| *sessions > 0)?,
            maintenance.remem_minutes_per_100_sessions?,
            maintenance.reduction_pct?,
            source?,
        ))
    });
    let Some((
        authority,
        curator_minutes,
        curated_rate,
        remem_sessions,
        remem_rate,
        reduction,
        source,
    )) = measured
    else {
        return unavailable_field(
            "maintenance_time_and_ai_usage",
            "official GH931 target-blind curator and remem_e2e runs",
            "maintenance minutes and AI usage units",
            "100 eligible sessions and official task runs",
            "governed maintenance-time evidence is incomplete",
        );
    };
    let mut values = BTreeMap::new();
    values.insert("curated_minutes_per_100_sessions".to_string(), curated_rate);
    values.insert("remem_minutes_per_100_sessions".to_string(), remem_rate);
    values.insert("maintenance_reduction_pct".to_string(), reduction);
    ScorecardField {
        id: "maintenance_time_and_ai_usage",
        measurement_state: MeasurementState::Measured,
        eligible_population: "official GH931 target-blind curator and remem_e2e sessions"
            .to_string(),
        numerator: ScorecardComponent {
            definition: "target-blind curator maintenance minutes".to_string(),
            value: Some(curator_minutes),
        },
        denominator: ScorecardComponent {
            definition: "eligible curator sessions".to_string(),
            value: Some(authority.maintenance.curator_sessions as f64),
        },
        values,
        threshold: "registered GH931 maintenance-reduction threshold".to_string(),
        source: Some(source),
        claim_level: gh931_claim_level(Some(authority)).to_string(),
        note: format!(
            "Maintenance time is verifier-measured across {} remem sessions; AI usage remains unavailable.",
            remem_sessions
        ),
    }
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
    let summary = verified_security_summary(public);
    let denominator = summary.map(|summary| summary.non_retention_cases as f64);
    let rate = summary.map(|summary| summary.non_retention_leak_rate);
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

fn abstention_field(_public: &PublicEvidence, _source: Option<String>) -> ScorecardField {
    unavailable_field(
        "abstention_rate",
        "production-path adversarial-policy v2 runs",
        "runs ending in policy abstention",
        "verified adversarial runs",
        "the recomputed policy summary records abstention applicability/accuracy, not an authoritative abstention event count",
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

fn gh931_claim_level(
    authority: Option<&crate::eval::bench_artifact::Gh931AuthorityVerdict>,
) -> &'static str {
    if authority.is_some_and(|authority| authority.status == AuthorityStatus::Pass) {
        "level_2_registered_coding_outcome_claim"
    } else {
        "unavailable_no_authorized_coding_claim"
    }
}

fn verified_security_summary(
    public: &PublicEvidence,
) -> Option<&crate::eval::memory_bench::types::MemoryBenchPolicySummary> {
    let authority = public.security_authority.as_ref()?;
    if authority.status != AuthorityStatus::Pass
        || authority.target.is_none()
        || authority.models.is_empty()
        || authority.platforms.len() != 1
        || authority.report_sha256.len() != 64
    {
        return None;
    }
    authority.policy_summary.as_ref()
}
