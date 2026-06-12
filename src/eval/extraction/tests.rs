use super::run::{evaluate_corpus, run_corpus_path};
use super::types::{
    CandidateExpectation, ExtractionCase, ExtractionCorpus, ExtractionEvalOptions,
    ExtractionRateMetric, ObservationExpectation, TranscriptEvent,
};

#[test]
fn committed_corpus_scores_current_baseline() {
    let report = run_corpus_path(ExtractionEvalOptions::default()).unwrap();

    assert!(report.metrics.all_checks_passed);
    assert_eq!(report.metadata.cases, 2);
    assert_eq!(
        report.metrics.observation_precision,
        ExtractionRateMetric::new(2, 2)
    );
    assert_eq!(
        report.metrics.observation_recall,
        ExtractionRateMetric::new(2, 2)
    );
    assert_eq!(
        report.metrics.candidate_precision,
        ExtractionRateMetric::new(2, 2)
    );
    assert_eq!(
        report.metrics.candidate_recall,
        ExtractionRateMetric::new(2, 2)
    );
    assert_eq!(report.metrics.over_saved_predictions, 0);
    assert!(report
        .cases
        .iter()
        .all(|case| case.observation_request_sha256.len() == 64
            && case.candidate_request_sha256.len() == 64));
    assert!(report.failing_examples.is_empty());
}

#[test]
fn detects_over_saved_observations_and_candidates() {
    let corpus = ExtractionCorpus {
        version: "test".to_string(),
        description: "inline over-save fixture".to_string(),
        cases: vec![ExtractionCase {
            id: "over-save".to_string(),
            transcript: vec![TranscriptEvent {
                id: "evt-1".to_string(),
                role: "user".to_string(),
                content: "Remember the verified build loop rule only.".to_string(),
                tool_name: None,
                event_type: None,
                token_estimate: None,
                created_at_epoch: None,
            }],
            observation_output: serde_json::json!({
                "observations": [
                    {
                        "type": "decision",
                        "title": "Verified build loop rule",
                        "subtitle": null,
                        "narrative": "Keep the verified build loop rule.",
                        "facts": [],
                        "concepts": [],
                        "files_read": [],
                        "files_modified": [],
                        "confidence": 0.9
                    },
                    {
                        "type": "discovery",
                        "title": "Unsupported preference",
                        "subtitle": null,
                        "narrative": "Invent an unsupported preference.",
                        "facts": [],
                        "concepts": [],
                        "files_read": [],
                        "files_modified": [],
                        "confidence": 0.7
                    }
                ]
            })
            .to_string(),
            candidate_output: "<memory_candidate>\n<scope>project</scope>\n<type>lesson</type>\n<topic_key>verified-build-loop-rule</topic_key>\n<risk_class>medium</risk_class>\n<confidence>0.85</confidence>\n<text>Keep the verified build loop rule.</text>\n</memory_candidate>\n<memory_candidate>\n<scope>project</scope>\n<type>preference</type>\n<topic_key>unsupported-preference</topic_key>\n<risk_class>high</risk_class>\n<confidence>0.8</confidence>\n<text>Invent an unsupported preference.</text>\n</memory_candidate>".to_string(),
            expected_observations: vec![ObservationExpectation {
                id: "obs-rule".to_string(),
                observation_type: Some("decision".to_string()),
                text_contains: vec!["verified build loop rule".to_string()],
            }],
            forbidden_observations: vec![],
            expected_candidates: vec![CandidateExpectation {
                id: "cand-rule".to_string(),
                scope: Some("project".to_string()),
                memory_type: Some("lesson".to_string()),
                topic_key: Some("verified-build-loop-rule".to_string()),
                risk_class: Some("medium".to_string()),
                text_contains: vec!["verified build loop rule".to_string()],
            }],
            forbidden_candidates: vec![],
        }],
    };

    let report = evaluate_corpus("inline", &corpus).unwrap();

    assert!(!report.metrics.all_checks_passed);
    assert_eq!(
        report.metrics.observation_precision,
        ExtractionRateMetric::new(1, 2)
    );
    assert_eq!(
        report.metrics.candidate_precision,
        ExtractionRateMetric::new(1, 2)
    );
    assert_eq!(report.metrics.over_saved_predictions, 2);
    assert_eq!(report.metrics.total_predictions, 4);
    assert_eq!(report.metrics.over_save_penalty, 0.5);
}
