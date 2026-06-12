use std::fs;
use std::path::Path;

use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

use super::types::{
    CandidateExpectation, CandidatePrediction, ExtractionCase, ExtractionCaseReport,
    ExtractionCorpus, ExtractionEvalMetadata, ExtractionEvalOptions, ExtractionEvalReport,
    ExtractionMetricSummary, ExtractionRateMetric, ObservationExpectation, ObservationPrediction,
};

const EVAL_PROJECT: &str = "/tmp/remem/extraction-eval";
const EVAL_HOST: &str = "codex-cli";
const EVAL_SESSION_ID: &str = "extraction-eval-session";
const DEFAULT_OBSERVATION_CONFIDENCE: f64 = 0.75;

pub(crate) fn load_corpus(path: &str) -> Result<ExtractionCorpus> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read extraction eval corpus {}", Path::new(path).display()))?;
    let corpus: ExtractionCorpus = serde_json::from_str(&content)
        .with_context(|| format!("parse extraction eval corpus {}", Path::new(path).display()))?;
    validate_corpus(&corpus)?;
    Ok(corpus)
}

pub fn run_corpus_path(options: ExtractionEvalOptions) -> Result<ExtractionEvalReport> {
    let corpus = load_corpus(&options.corpus_path)?;
    evaluate_corpus(options.corpus_path.as_str(), &corpus)
}

pub(crate) fn evaluate_corpus(
    corpus_path: &str,
    corpus: &ExtractionCorpus,
) -> Result<ExtractionEvalReport> {
    let cases: Vec<ExtractionCaseReport> = corpus
        .cases
        .iter()
        .map(evaluate_case)
        .collect::<Result<Vec<_>>>()?;
    let metrics = summarize_metrics(corpus, &cases);
    let failing_examples = collect_failures(&cases);
    let transcript_events = corpus
        .cases
        .iter()
        .map(|case| case.transcript.len())
        .sum::<usize>();
    Ok(ExtractionEvalReport {
        metadata: ExtractionEvalMetadata {
            corpus: stable_corpus_path(corpus_path),
            corpus_version: corpus.version.clone(),
            description: corpus.description.clone(),
            cases: cases.len(),
            transcript_events,
        },
        metrics: ExtractionMetricSummary {
            all_checks_passed: failing_examples.is_empty(),
            ..metrics
        },
        cases,
        failing_examples,
    })
}

fn validate_corpus(corpus: &ExtractionCorpus) -> Result<()> {
    ensure!(
        !corpus.version.trim().is_empty(),
        "extraction eval corpus version is required"
    );
    ensure!(
        !corpus.cases.is_empty(),
        "extraction eval corpus must contain at least one case"
    );
    for case in &corpus.cases {
        ensure!(
            !case.id.trim().is_empty(),
            "extraction eval case id is required"
        );
        ensure!(
            !case.transcript.is_empty(),
            "extraction eval case {} must include transcript events",
            case.id
        );
        ensure!(
            !case.expected_observations.is_empty() || !case.expected_candidates.is_empty(),
            "extraction eval case {} must include at least one expected label",
            case.id
        );
        for event in &case.transcript {
            ensure!(
                !event.id.trim().is_empty()
                    && !event.role.trim().is_empty()
                    && !event.content.trim().is_empty(),
                "extraction eval case {} has an incomplete transcript event",
                case.id
            );
            if let Some(tool_name) = event.tool_name.as_deref() {
                ensure!(
                    !tool_name.trim().is_empty(),
                    "extraction eval case {} has an empty tool_name",
                    case.id
                );
            }
        }
        for expected in &case.expected_observations {
            validate_observation_ref(&case.id, "expected_observations", expected)?;
        }
        for forbidden in &case.forbidden_observations {
            validate_observation_ref(&case.id, "forbidden_observations", forbidden)?;
        }
        for expected in &case.expected_candidates {
            validate_candidate_ref(&case.id, "expected_candidates", expected)?;
        }
        for forbidden in &case.forbidden_candidates {
            validate_candidate_ref(&case.id, "forbidden_candidates", forbidden)?;
        }
    }
    Ok(())
}

fn validate_observation_ref(
    case_id: &str,
    field: &str,
    reference: &ObservationExpectation,
) -> Result<()> {
    ensure!(
        !reference.id.trim().is_empty(),
        "extraction eval case {case_id} {field} id is required"
    );
    ensure!(
        reference.observation_type.is_some() || !reference.text_contains.is_empty(),
        "extraction eval case {case_id} {field} {} needs a matcher",
        reference.id
    );
    Ok(())
}

fn validate_candidate_ref(
    case_id: &str,
    field: &str,
    reference: &CandidateExpectation,
) -> Result<()> {
    ensure!(
        !reference.id.trim().is_empty(),
        "extraction eval case {case_id} {field} id is required"
    );
    ensure!(
        reference.scope.is_some()
            || reference.memory_type.is_some()
            || reference.topic_key.is_some()
            || reference.risk_class.is_some()
            || !reference.text_contains.is_empty(),
        "extraction eval case {case_id} {field} {} needs a matcher",
        reference.id
    );
    Ok(())
}

fn evaluate_case(case: &ExtractionCase) -> Result<ExtractionCaseReport> {
    let predicted_observations = parse_observation_predictions(&case.observation_output);
    let predicted_candidates = parse_candidate_predictions(&case.candidate_output)?;
    let observation_request_sha256 = sha256_hex(&build_observation_request(case));
    let candidate_request_sha256 =
        sha256_hex(&build_candidate_request(case, &predicted_observations));

    let observation_match =
        match_observations(&predicted_observations, &case.expected_observations);
    let candidate_match = match_candidates(&predicted_candidates, &case.expected_candidates);
    let forbidden_observations =
        forbidden_observation_hits(&predicted_observations, &case.forbidden_observations);
    let forbidden_candidates =
        forbidden_candidate_hits(&predicted_candidates, &case.forbidden_candidates);
    let over_saved_predictions =
        observation_match.unexpected.len() + candidate_match.unexpected.len();
    let pass = observation_match.missing.is_empty()
        && observation_match.unexpected.is_empty()
        && forbidden_observations.is_empty()
        && candidate_match.missing.is_empty()
        && candidate_match.unexpected.is_empty()
        && forbidden_candidates.is_empty();

    Ok(ExtractionCaseReport {
        id: case.id.clone(),
        transcript_events: case.transcript.len(),
        observation_request_sha256,
        candidate_request_sha256,
        predicted_observations,
        predicted_candidates,
        missing_expected_observations: observation_match.missing,
        unexpected_observations: observation_match.unexpected,
        forbidden_observations,
        missing_expected_candidates: candidate_match.missing,
        unexpected_candidates: candidate_match.unexpected,
        forbidden_candidates,
        over_saved_predictions,
        pass,
    })
}

fn parse_observation_predictions(output: &str) -> Vec<ObservationPrediction> {
    crate::memory::format::parse_observations(output)
        .into_iter()
        .enumerate()
        .map(|(index, observation)| ObservationPrediction {
            index,
            observation_type: observation.obs_type.clone(),
            text: crate::observation_extract::observation_text(&observation),
            confidence: observation.confidence,
        })
        .collect()
}

fn parse_candidate_predictions(output: &str) -> Result<Vec<CandidatePrediction>> {
    Ok(crate::memory_candidate::parse_candidate_output(output)?
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| CandidatePrediction {
            index,
            scope: candidate.scope,
            memory_type: candidate.memory_type,
            topic_key: candidate.topic_key,
            risk_class: candidate.risk_class,
            text: candidate.text,
        })
        .collect())
}

struct MatchOutcome {
    missing: Vec<String>,
    unexpected: Vec<usize>,
}

fn match_observations(
    predictions: &[ObservationPrediction],
    expected: &[ObservationExpectation],
) -> MatchOutcome {
    let mut used_predictions = vec![false; predictions.len()];
    let mut missing = Vec::new();
    for reference in expected {
        if let Some(index) = predictions
            .iter()
            .enumerate()
            .position(|(index, prediction)| {
                !used_predictions[index] && observation_matches(prediction, reference)
            })
        {
            used_predictions[index] = true;
        } else {
            missing.push(reference.id.clone());
        }
    }
    let unexpected = used_predictions
        .iter()
        .enumerate()
        .filter_map(|(index, used)| (!*used).then_some(index))
        .collect::<Vec<_>>();
    MatchOutcome {
        missing,
        unexpected,
    }
}

fn match_candidates(
    predictions: &[CandidatePrediction],
    expected: &[CandidateExpectation],
) -> MatchOutcome {
    let mut used_predictions = vec![false; predictions.len()];
    let mut missing = Vec::new();
    for reference in expected {
        if let Some(index) = predictions
            .iter()
            .enumerate()
            .position(|(index, prediction)| {
                !used_predictions[index] && candidate_matches(prediction, reference)
            })
        {
            used_predictions[index] = true;
        } else {
            missing.push(reference.id.clone());
        }
    }
    let unexpected = used_predictions
        .iter()
        .enumerate()
        .filter_map(|(index, used)| (!*used).then_some(index))
        .collect::<Vec<_>>();
    MatchOutcome {
        missing,
        unexpected,
    }
}

fn forbidden_observation_hits(
    predictions: &[ObservationPrediction],
    forbidden: &[ObservationExpectation],
) -> Vec<String> {
    forbidden
        .iter()
        .filter(|reference| {
            predictions
                .iter()
                .any(|prediction| observation_matches(prediction, reference))
        })
        .map(|reference| reference.id.clone())
        .collect()
}

fn forbidden_candidate_hits(
    predictions: &[CandidatePrediction],
    forbidden: &[CandidateExpectation],
) -> Vec<String> {
    forbidden
        .iter()
        .filter(|reference| {
            predictions
                .iter()
                .any(|prediction| candidate_matches(prediction, reference))
        })
        .map(|reference| reference.id.clone())
        .collect()
}

fn observation_matches(
    prediction: &ObservationPrediction,
    reference: &ObservationExpectation,
) -> bool {
    if let Some(expected_type) = reference.observation_type.as_deref() {
        if !prediction
            .observation_type
            .eq_ignore_ascii_case(expected_type.trim())
        {
            return false;
        }
    }
    contains_all(&prediction.text, &reference.text_contains)
}

fn candidate_matches(prediction: &CandidatePrediction, reference: &CandidateExpectation) -> bool {
    field_matches(reference.scope.as_deref(), &prediction.scope)
        && field_matches(reference.memory_type.as_deref(), &prediction.memory_type)
        && field_matches(reference.topic_key.as_deref(), &prediction.topic_key)
        && field_matches(reference.risk_class.as_deref(), &prediction.risk_class)
        && contains_all(&prediction.text, &reference.text_contains)
}

fn field_matches(expected: Option<&str>, actual: &str) -> bool {
    expected
        .map(|value| actual.eq_ignore_ascii_case(value.trim()))
        .unwrap_or(true)
}

fn contains_all(text: &str, needles: &[String]) -> bool {
    let haystack = text.to_ascii_lowercase();
    needles
        .iter()
        .all(|needle| haystack.contains(&needle.to_ascii_lowercase()))
}

fn summarize_metrics(
    corpus: &ExtractionCorpus,
    cases: &[ExtractionCaseReport],
) -> ExtractionMetricSummary {
    let observation_predictions = cases
        .iter()
        .map(|case| case.predicted_observations.len())
        .sum::<usize>();
    let candidate_predictions = cases
        .iter()
        .map(|case| case.predicted_candidates.len())
        .sum::<usize>();
    let over_saved_predictions = cases
        .iter()
        .map(|case| case.over_saved_predictions)
        .sum::<usize>();
    let total_predictions = observation_predictions + candidate_predictions;
    let observation_expected = corpus
        .cases
        .iter()
        .map(|case| case.expected_observations.len())
        .sum::<usize>();
    let candidate_expected = corpus
        .cases
        .iter()
        .map(|case| case.expected_candidates.len())
        .sum::<usize>();
    let observation_matched = observation_predictions
        - cases
            .iter()
            .map(|case| case.unexpected_observations.len())
            .sum::<usize>();
    let candidate_matched = candidate_predictions
        - cases
            .iter()
            .map(|case| case.unexpected_candidates.len())
            .sum::<usize>();
    let forbidden_observation_hits = cases
        .iter()
        .map(|case| case.forbidden_observations.len())
        .sum::<usize>();
    let forbidden_candidate_hits = cases
        .iter()
        .map(|case| case.forbidden_candidates.len())
        .sum::<usize>();
    let forbidden_observation_total = corpus
        .cases
        .iter()
        .map(|case| case.forbidden_observations.len())
        .sum::<usize>();
    let forbidden_candidate_total = corpus
        .cases
        .iter()
        .map(|case| case.forbidden_candidates.len())
        .sum::<usize>();

    ExtractionMetricSummary {
        observation_precision: ExtractionRateMetric::new(
            observation_matched,
            observation_predictions,
        ),
        observation_recall: ExtractionRateMetric::new(
            observation_expected - missing_observations(cases),
            observation_expected,
        ),
        candidate_precision: ExtractionRateMetric::new(candidate_matched, candidate_predictions),
        candidate_recall: ExtractionRateMetric::new(
            candidate_expected - missing_candidates(cases),
            candidate_expected,
        ),
        forbidden_observation_exclusion: ExtractionRateMetric::new(
            forbidden_observation_total - forbidden_observation_hits,
            forbidden_observation_total,
        ),
        forbidden_candidate_exclusion: ExtractionRateMetric::new(
            forbidden_candidate_total - forbidden_candidate_hits,
            forbidden_candidate_total,
        ),
        over_saved_predictions,
        total_predictions,
        over_save_penalty: if total_predictions == 0 {
            0.0
        } else {
            over_saved_predictions as f64 / total_predictions as f64
        },
        all_checks_passed: false,
    }
}

fn missing_observations(cases: &[ExtractionCaseReport]) -> usize {
    cases
        .iter()
        .map(|case| case.missing_expected_observations.len())
        .sum()
}

fn missing_candidates(cases: &[ExtractionCaseReport]) -> usize {
    cases
        .iter()
        .map(|case| case.missing_expected_candidates.len())
        .sum()
}

fn collect_failures(cases: &[ExtractionCaseReport]) -> Vec<String> {
    let mut failures = Vec::new();
    for case in cases {
        for missing in &case.missing_expected_observations {
            failures.push(format!(
                "{} missing expected observation {}",
                case.id, missing
            ));
        }
        for unexpected in &case.unexpected_observations {
            failures.push(format!(
                "{} unexpected observation prediction {}",
                case.id, unexpected
            ));
        }
        for forbidden in &case.forbidden_observations {
            failures.push(format!(
                "{} saved forbidden observation {}",
                case.id, forbidden
            ));
        }
        for missing in &case.missing_expected_candidates {
            failures.push(format!(
                "{} missing expected candidate {}",
                case.id, missing
            ));
        }
        for unexpected in &case.unexpected_candidates {
            failures.push(format!(
                "{} unexpected candidate prediction {}",
                case.id, unexpected
            ));
        }
        for forbidden in &case.forbidden_candidates {
            failures.push(format!(
                "{} saved forbidden candidate {}",
                case.id, forbidden
            ));
        }
    }
    failures
}

fn build_observation_request(case: &ExtractionCase) -> String {
    let events = case
        .transcript
        .iter()
        .enumerate()
        .map(
            |(index, event)| crate::observation_extract::ObservationPromptEvent {
                id: (index + 1) as i64,
                event_type: event.event_type.as_deref().unwrap_or_else(|| {
                    if event.tool_name.is_some() {
                        "tool_result"
                    } else {
                        "message"
                    }
                }),
                role: Some(event.role.as_str()),
                tool_name: event.tool_name.as_deref(),
                content: event.content.as_str(),
                token_estimate: event
                    .token_estimate
                    .unwrap_or_else(|| estimate_tokens(&event.content)),
                created_at_epoch: event.created_at_epoch.unwrap_or((index + 1) as i64),
            },
        )
        .collect::<Vec<_>>();
    crate::observation_extract::build_eval_extract_request(
        EVAL_PROJECT,
        EVAL_HOST,
        Some(EVAL_SESSION_ID),
        &events,
    )
}

fn build_candidate_request(
    case: &ExtractionCase,
    observations: &[ObservationPrediction],
) -> String {
    let event_ids = (1..=case.transcript.len() as i64).collect::<Vec<_>>();
    let prompt_observations = observations
        .iter()
        .map(
            |observation| crate::memory_candidate::CandidatePromptObservation {
                id: (observation.index + 1) as i64,
                observation_type: observation.observation_type.as_str(),
                text: observation.text.as_str(),
                evidence_event_ids: event_ids.clone(),
                confidence: observation
                    .confidence
                    .unwrap_or(DEFAULT_OBSERVATION_CONFIDENCE),
            },
        )
        .collect::<Vec<_>>();
    crate::memory_candidate::build_eval_candidate_request(
        EVAL_PROJECT,
        EVAL_HOST,
        Some(EVAL_SESSION_ID),
        &prompt_observations,
    )
}

fn estimate_tokens(content: &str) -> i64 {
    ((content.len() as i64) + 3) / 4
}

fn stable_corpus_path(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    while let Some(stripped) = normalized.strip_prefix("./") {
        normalized = stripped.to_string();
    }
    normalized
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
