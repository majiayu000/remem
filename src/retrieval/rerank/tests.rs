use std::collections::HashSet;

use anyhow::Result;

use crate::memory::Memory;

use super::config::{validate_config, RerankConfig};
use super::inventory::{
    install_dir_for_preset, inventory_state, write_test_manifest, RerankerInventoryState,
    RerankerPreset,
};
use super::stage::{order_scored_candidates, run_stage, truncate_utf8};
use super::types::{RerankCandidate, RerankDisabledReason, RerankStatus};
use super::{build_candidates, candidate_document, reorder_applied};

fn enabled_config(model_dir: Option<&std::path::Path>) -> RerankConfig {
    RerankConfig {
        enabled: true,
        model_dir: model_dir.map(|dir| dir.display().to_string()),
        ..RerankConfig::default()
    }
}

fn candidate(id: i64, baseline_rank: usize, verify_before_trust: bool) -> RerankCandidate {
    RerankCandidate {
        id,
        baseline_rank,
        verify_before_trust,
        document: format!("doc {id}"),
    }
}

fn test_memory(id: i64) -> Memory {
    Memory {
        id,
        session_id: None,
        project: "proj".to_string(),
        topic_key: None,
        title: format!("Memory {id}"),
        text: format!("content {id}"),
        memory_type: "decision".to_string(),
        files: None,
        created_at_epoch: 100,
        updated_at_epoch: 100,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

struct TempModelRoot(std::path::PathBuf);

impl TempModelRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "remem-rerank-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp model root");
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempModelRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const TEST_ROLE_FILES: &[(&str, &[u8])] = &[
    ("onnx/model.onnx", b"onnx-bytes"),
    ("tokenizer.json", b"{}"),
    ("config.json", b"{}"),
    ("special_tokens_map.json", b"{}"),
    ("tokenizer_config.json", b"{}"),
];

#[test]
fn rerank_config_defaults_keep_rerank_disabled() {
    let config = RerankConfig::default();

    assert!(!config.enabled);
    assert!(config.top_n >= config.top_k);
    assert!(validate_config(&config).is_ok());
}

#[test]
fn rerank_config_rejects_top_n_smaller_than_top_k() {
    let config = RerankConfig {
        top_n: 5,
        top_k: 10,
        ..RerankConfig::default()
    };

    let error = validate_config(&config).unwrap_err();

    assert!(error.to_string().contains("top_n"));
}

#[test]
fn rerank_config_rejects_zero_top_k_and_deadline() {
    let zero_k = RerankConfig {
        top_k: 0,
        ..RerankConfig::default()
    };
    let zero_deadline = RerankConfig {
        deadline_ms: 0,
        ..RerankConfig::default()
    };

    assert!(validate_config(&zero_k).is_err());
    assert!(validate_config(&zero_deadline).is_err());
}

#[test]
fn rerank_config_rejects_unknown_preset() {
    let config = RerankConfig {
        preset: "ms-marco-minilm".to_string(),
        ..RerankConfig::default()
    };

    let error = validate_config(&config).unwrap_err();

    assert!(error.to_string().contains("unsupported reranker preset"));
}

#[test]
fn rerank_off_is_baseline_equivalent() {
    let config = RerankConfig::default();
    let candidates = vec![candidate(1, 0, false), candidate(2, 1, false)];

    let outcome = run_stage(&config, "query", &candidates);

    assert_eq!(
        outcome.status,
        RerankStatus::NotApplied {
            disabled_reason: RerankDisabledReason::Off
        }
    );
    assert!(outcome.ordered_ids.is_empty());
}

#[test]
fn rerank_empty_query_is_not_applied_without_model_load() {
    let config = enabled_config(None);

    let outcome = run_stage(&config, "   ", &[candidate(1, 0, false)]);

    assert_eq!(
        outcome.disabled_reason(),
        Some(RerankDisabledReason::EmptyQuery)
    );
}

#[test]
fn rerank_empty_candidates_is_empty_applied_outcome() {
    // B-002: zero candidates is a normal empty outcome; the model is never
    // loaded and no pseudo error is reported.
    let temp = TempModelRoot::new("empty");
    let config = enabled_config(Some(temp.path()));

    let outcome = run_stage(&config, "query", &[]);

    assert!(outcome.applied());
    assert!(outcome.ordered_ids.is_empty());
    assert_eq!(outcome.input_count, 0);
    assert_eq!(outcome.output_count, 0);
}

#[test]
fn rerank_missing_or_corrupt_is_fail_visible() -> Result<()> {
    let temp = TempModelRoot::new("failvisible");
    let config = enabled_config(Some(temp.path()));
    let candidates = vec![candidate(1, 0, false)];

    // Missing manifest: stable model_missing reason, full baseline retained.
    let outcome = run_stage(&config, "query", &candidates);
    assert_eq!(
        outcome.disabled_reason(),
        Some(RerankDisabledReason::ModelMissing)
    );

    // Corrupt inventory: tamper with a verified file after manifest publish.
    let install_dir = install_dir_for_preset(&config, RerankerPreset::BgeRerankerBase)?;
    write_test_manifest(
        &install_dir,
        RerankerPreset::BgeRerankerBase,
        TEST_ROLE_FILES,
    )?;
    std::fs::write(install_dir.join("tokenizer.json"), b"{\"tampered\":true}")?;
    let outcome = run_stage(&config, "query", &candidates);
    assert_eq!(
        outcome.disabled_reason(),
        Some(RerankDisabledReason::ModelCorrupt)
    );
    Ok(())
}

#[test]
fn reranker_inventory_publish_is_verified() -> Result<()> {
    let temp = TempModelRoot::new("inventory");
    let config = enabled_config(Some(temp.path()));
    let install_dir = install_dir_for_preset(&config, RerankerPreset::BgeRerankerBase)?;
    write_test_manifest(
        &install_dir,
        RerankerPreset::BgeRerankerBase,
        TEST_ROLE_FILES,
    )?;

    let state = inventory_state(&config)?;

    match state {
        RerankerInventoryState::Ready(verified) => {
            assert_eq!(verified.manifest.preset, "bge-reranker-base");
            assert_eq!(verified.manifest_sha256.len(), 64);
        }
        other => panic!("expected verified inventory, got {other:?}"),
    }

    // Deleting a verified file must flip the state to corrupt, not ready.
    std::fs::remove_file(install_dir.join("config.json"))?;
    let state = inventory_state(&config)?;
    assert!(matches!(state, RerankerInventoryState::Corrupt(_)));
    Ok(())
}

#[test]
fn rerank_shared_stage_top_n_membership() {
    // Only the fixed top-N baseline candidates may appear in the output.
    let candidates: Vec<&RerankCandidate> = vec![];
    let empty = order_scored_candidates(&candidates, &[], 5);
    assert!(empty.is_empty());

    let all = [
        candidate(10, 0, false),
        candidate(20, 1, false),
        candidate(30, 2, false),
    ];
    let refs: Vec<&RerankCandidate> = all.iter().collect();
    let ordered = order_scored_candidates(&refs, &[0.1, 0.9, 0.5], 2);

    // Fixed top-k cut: exactly k results, all from the input candidate set.
    assert_eq!(ordered, vec![20, 30]);
}

#[test]
fn rerank_empty_short_and_tie_break() {
    // Ties break by baseline rank ascending, then stable id ascending.
    let all = [
        candidate(30, 0, false),
        candidate(10, 1, false),
        candidate(20, 2, false),
    ];
    let refs: Vec<&RerankCandidate> = all.iter().collect();

    let ordered = order_scored_candidates(&refs, &[0.5, 0.5, 0.5], 3);

    assert_eq!(ordered, vec![30, 10, 20]);
}

#[test]
fn rerank_preserves_eligibility_and_source_anchor() {
    // Hard partition: verify-before-trust candidates rank behind every
    // normal candidate even with a higher model score, while each partition
    // keeps its internal rerank order.
    let all = [
        candidate(1, 0, true),
        candidate(2, 1, false),
        candidate(3, 2, false),
    ];
    let refs: Vec<&RerankCandidate> = all.iter().collect();

    let ordered = order_scored_candidates(&refs, &[0.99, 0.2, 0.8], 3);

    assert_eq!(ordered, vec![3, 2, 1]);
}

#[test]
fn rerank_fixed_result_pagination_contract() {
    // The applied outcome is one fixed top-k ordered set; a smaller candidate
    // pool is scored as-is (no disabled fallback for count < top_k).
    let all = [candidate(1, 0, false), candidate(2, 1, false)];
    let refs: Vec<&RerankCandidate> = all.iter().collect();

    let ordered = order_scored_candidates(&refs, &[0.3, 0.7], 10);

    assert_eq!(ordered, vec![2, 1]);
}

#[test]
fn rerank_disable_rollback_restores_baseline() {
    // reorder_applied leaves the baseline untouched for not-applied outcomes.
    let memories = vec![test_memory(1), test_memory(2)];
    let outcome = super::types::RerankOutcome::not_applied(RerankDisabledReason::ModelMissing);

    let ordered = reorder_applied(memories, &outcome);

    assert_eq!(ordered.iter().map(|m| m.id).collect::<Vec<_>>(), vec![1, 2]);
}

#[test]
fn rerank_applied_outcome_reorders_and_truncates() {
    let memories = vec![test_memory(1), test_memory(2), test_memory(3)];
    let outcome = super::types::RerankOutcome {
        status: RerankStatus::Applied,
        ordered_ids: vec![3, 1],
        preset: Some("bge-reranker-base".into()),
        model_manifest_sha256: Some("m".repeat(64)),
        input_count: 3,
        output_count: 2,
        top_n: 3,
        top_k: 2,
        timings: vec![],
    };

    let ordered = reorder_applied(memories, &outcome);

    assert_eq!(ordered.iter().map(|m| m.id).collect::<Vec<_>>(), vec![3, 1]);
}

#[test]
fn candidate_projection_is_deterministic_and_bounded() {
    let memory = test_memory(7);

    let document = candidate_document(&memory);

    assert_eq!(
        document,
        "title: Memory 7\ntype: decision\ncontent: content 7"
    );

    // UTF-8 boundary-safe truncation only affects the model input.
    let text = "记忆内容测试";
    for max in 0..text.len() {
        let truncated = truncate_utf8(text, max);
        assert!(truncated.len() <= max);
        assert!(text.starts_with(&truncated));
    }
    assert_eq!(truncate_utf8(text, text.len()), text);
}

#[test]
fn build_candidates_assigns_stable_baseline_ranks_and_vbt_flags() {
    let memories = vec![test_memory(5), test_memory(6)];
    let vbt: HashSet<i64> = [6].into_iter().collect();

    let candidates = build_candidates(&memories, &vbt);

    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].id, 5);
    assert_eq!(candidates[0].baseline_rank, 0);
    assert!(!candidates[0].verify_before_trust);
    assert_eq!(candidates[1].id, 6);
    assert_eq!(candidates[1].baseline_rank, 1);
    assert!(candidates[1].verify_before_trust);
}

#[test]
fn rerank_disabled_reason_tokens_are_stable() {
    let cases = [
        (RerankDisabledReason::Off, "off", false),
        (RerankDisabledReason::EmptyQuery, "empty_query", false),
        (RerankDisabledReason::ModelMissing, "model_missing", true),
        (RerankDisabledReason::ModelCorrupt, "model_corrupt", true),
        (
            RerankDisabledReason::ModelLoadFailed,
            "model_load_failed",
            true,
        ),
        (
            RerankDisabledReason::InferenceFailed,
            "inference_failed",
            true,
        ),
        (
            RerankDisabledReason::DeadlineExceeded,
            "deadline_exceeded",
            true,
        ),
        (RerankDisabledReason::Cancelled, "cancelled", true),
    ];
    for (reason, token, is_error) in cases {
        assert_eq!(reason.as_str(), token);
        assert_eq!(reason.is_error(), is_error);
    }
}

#[test]
fn rerank_diagnostics_contract() {
    let outcome = super::types::RerankOutcome::not_applied(RerankDisabledReason::ModelMissing);

    let explain = outcome.to_explain(true);

    assert!(explain.requested);
    assert!(!explain.applied);
    assert_eq!(explain.disabled_reason.as_deref(), Some("model_missing"));
    let json = serde_json::to_value(&explain).expect("serialize");
    assert_eq!(json["disabled_reason"], "model_missing");
    assert_eq!(json["requested"], true);
    assert_eq!(json["applied"], false);
}
