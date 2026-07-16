use super::decision::{existing_slices_within_budget, provider_row};
use super::*;
use crate::eval::golden::{EvidenceRef, GoldenMemory, GoldenQuery, MetricAverages};
use std::collections::BTreeMap;

#[test]
fn provider_comparison_reports_required_rows_without_api_or_local_model() -> Result<()> {
    with_clean_provider_env(|| {
        let dataset = small_provider_dataset();
        let options = ProviderComparisonOptions {
            dataset_path: "test-provider-comparison.json".to_string(),
            k: 5,
            json_out: "/tmp/remem-provider-comparison-test.json".to_string(),
            allow_api: false,
        };
        let report = run_provider_comparison_dataset_locked(options, dataset)?;

        assert_eq!(report.providers.len(), 3);
        let feature_hash = provider_row(&report.providers, "feature-hash")
            .context("feature-hash row should exist")?;
        let local = provider_row(&report.providers, "local").context("local row should exist")?;
        let api = provider_row(&report.providers, "api").context("api row should exist")?;

        assert!(feature_hash.available);
        assert_eq!(
            feature_hash
                .provider_comparison_slice
                .as_ref()
                .map(|slice| slice.scored_queries),
            Some(1)
        );
        assert!(!local.available);
        assert!(local
            .unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .contains("local embedding model"));
        assert_eq!(
            local.model_id.as_deref(),
            Some("fastembed-intfloat-multilingual-e5-small-v1")
        );
        assert!(!api.available);
        assert!(api
            .unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .contains("--allow-api"));
        assert!(!report.default_decision.change_default);
        assert_eq!(
            report.default_decision.decision,
            DefaultDecisionKind::KeepFeatureHash
        );
        Ok(())
    })
}

#[test]
fn existing_slice_budget_checks_each_slice_not_only_aggregate() {
    let baseline = row_with_existing_slice_scores(&[("paraphrase", 1.0), ("temporal", 1.0)]);
    let same = row_with_existing_slice_scores(&[("paraphrase", 1.0), ("temporal", 1.0)]);
    let regressed = row_with_existing_slice_scores(&[("paraphrase", 1.0), ("temporal", 0.0)]);
    let mut baseline_with_abstention = baseline.clone();
    baseline_with_abstention
        .existing_slice_details
        .insert("abstention".to_string(), category_without_metrics());

    assert!(existing_slices_within_budget(&baseline, &same));
    assert!(existing_slices_within_budget(
        &baseline_with_abstention,
        &same
    ));
    assert!(!existing_slices_within_budget(&baseline, &regressed));
}

#[test]
fn feature_hash_provider_failures_are_not_reported_as_unavailable() {
    let error =
        ensure_optional_provider(EmbeddingProvider::FeatureHash, "synthetic baseline failure")
            .expect_err("feature-hash baseline failure must fail the report");

    assert!(error
        .to_string()
        .contains("feature-hash provider comparison baseline must be runnable"));
}

#[test]
fn optional_provider_probe_failures_become_unavailable_rows() -> Result<()> {
    let status = EmbeddingProviderStatus {
        configured_provider: "api".to_string(),
        fallback_provider: None,
        active_provider: "api".to_string(),
        active_model_id: Some("configured-model".to_string()),
        active_dimensions: Some(1536),
        degraded: false,
        disabled: false,
        unavailable_reason: None,
        degradation_reason: None,
        model_dir: None,
    };
    let row = optional_provider_error_row(
        EmbeddingProvider::OpenAi,
        &EmbeddingConfig::default(),
        status,
        "provider profile probe failed: synthetic probe rejection".to_string(),
        true,
    )?;

    assert!(!row.available);
    assert!(row
        .unavailable_reason
        .as_deref()
        .unwrap_or_default()
        .contains("synthetic probe rejection"));
    Ok(())
}

#[test]
fn forced_provider_config_resets_provider_specific_model_fields() {
    let defaults = EmbeddingConfig::default();
    let mut local_base = defaults.clone();
    local_base.provider = EmbeddingProvider::Local;
    local_base.model = "multilingual-e5-small".to_string();

    let api_config = forced_provider_config(&local_base, EmbeddingProvider::OpenAi);
    assert_eq!(api_config.provider, EmbeddingProvider::OpenAi);
    assert_eq!(api_config.model, defaults.model);
    assert_eq!(api_config.dimensions, defaults.dimensions);

    let mut api_base = EmbeddingConfig {
        provider: EmbeddingProvider::OpenAi,
        model: "custom-api-model".to_string(),
        dimensions: Some(1536),
        ..EmbeddingConfig::default()
    };
    api_base.fallback = Some(EmbeddingProvider::FeatureHash);

    let local_config = forced_provider_config(&api_base, EmbeddingProvider::Local);
    assert_eq!(local_config.provider, EmbeddingProvider::Local);
    assert_eq!(
        configured_model_id(EmbeddingProvider::Local, &local_config).as_deref(),
        Some("fastembed-intfloat-multilingual-e5-small-v1")
    );
    assert_eq!(local_config.dimensions, None);
    assert_eq!(local_config.fallback, None);
}

#[test]
fn available_rows_record_observed_embedding_profile() {
    let row = row_from_evaluation(
        EmbeddingProvider::OpenAi,
        &EmbeddingConfig::default(),
        EmbeddingProviderStatus {
            configured_provider: "api".to_string(),
            fallback_provider: None,
            active_provider: "api".to_string(),
            active_model_id: Some("configured-model".to_string()),
            active_dimensions: Some(1536),
            degraded: false,
            disabled: false,
            unavailable_reason: None,
            degradation_reason: None,
            model_dir: None,
        },
        "observed-model".to_string(),
        3072,
        empty_run_evaluation(),
        true,
    );

    assert_eq!(row.model_id.as_deref(), Some("observed-model"));
    assert_eq!(row.dimensions, Some(3072));
}

#[test]
fn committed_golden_dataset_contains_en_and_cjk_provider_comparison_cases() -> Result<()> {
    let dataset = golden::load_dataset(DEFAULT_DATASET_PATH)?;
    let cases = dataset
        .queries
        .iter()
        .filter(|query| query.slice_label() == PROVIDER_COMPARISON_SLICE)
        .collect::<Vec<_>>();
    assert!(
        cases.len() >= 4,
        "expected at least four provider_comparison cases"
    );
    assert!(cases.iter().any(|query| query.id.contains("-en-")));
    assert!(cases.iter().any(|query| !query.query.is_ascii()));
    Ok(())
}

fn empty_run_evaluation() -> ProviderRunEvaluation {
    ProviderRunEvaluation {
        overall: category_with_evidence(1.0),
        existing_slices: category_with_evidence(1.0),
        existing_slice_details: BTreeMap::new(),
        provider_comparison_slice: category_with_evidence(1.0),
        query_embedding_latencies_ms: vec![1.0],
        query_summaries: vec![],
    }
}

fn row_with_existing_slice_scores(scores: &[(&str, f64)]) -> ProviderComparisonRow {
    let existing_slice_details = scores
        .iter()
        .map(|(slice, score)| ((*slice).to_string(), category_with_evidence(*score)))
        .collect::<BTreeMap<_, _>>();
    ProviderComparisonRow {
        provider: "test",
        configured_provider: "test".to_string(),
        active_provider: "test".to_string(),
        fallback_provider: None,
        model_id: Some("test-model".to_string()),
        dimensions: Some(1),
        available: true,
        degraded: false,
        disabled: false,
        unavailable_reason: None,
        provider_config: ProviderConfigSummary {
            provider: "test".to_string(),
            fallback: None,
            model: "test-model".to_string(),
            base_url: "http://127.0.0.1".to_string(),
            dimensions: Some(1),
            api_key_env: "TEST_API_KEY".to_string(),
            model_dir: None,
            timeout_secs: 1,
            api_calls_allowed: false,
        },
        query_embedding_latency_p95_ms: Some(1.0),
        query_embedding_latency_samples: 1,
        overall: None,
        existing_slices: None,
        existing_slice_details,
        provider_comparison_slice: None,
        query_summaries: vec![],
    }
}

fn category_with_evidence(evidence_recall_at_k: f64) -> CategoryEvaluation {
    CategoryEvaluation {
        total_queries: 1,
        scored_queries: 1,
        abstention_queries: 0,
        abstention_passed: 0,
        query_tokens_per_query: 1.0,
        retrieval_latency_p50_ms: 1.0,
        retrieval_latency_p95_ms: 1.0,
        metrics: Some(MetricAverages {
            count: 1,
            hit_at_k: evidence_recall_at_k,
            mrr_at_10: evidence_recall_at_k,
            precision_at_k: evidence_recall_at_k,
            recall_at_k: evidence_recall_at_k,
            ndcg_at_10: evidence_recall_at_k,
            evidence_recall_at_k,
        }),
    }
}

fn category_without_metrics() -> CategoryEvaluation {
    CategoryEvaluation {
        total_queries: 1,
        scored_queries: 0,
        abstention_queries: 1,
        abstention_passed: 1,
        query_tokens_per_query: 1.0,
        retrieval_latency_p50_ms: 1.0,
        retrieval_latency_p95_ms: 1.0,
        metrics: None,
    }
}

fn small_provider_dataset() -> GoldenDataset {
    GoldenDataset {
        version: Some("provider-comparison-test".to_string()),
        description: Some("provider comparison test fixture".to_string()),
        corpus: vec![
            GoldenMemory {
                project: "synthetic/provider-test".to_string(),
                topic_key: Some("provider-test-target".to_string()),
                title: "Provider test target".to_string(),
                content: "Mira owns the violet cache recovery runbook.".to_string(),
                memory_type: "decision".to_string(),
                branch: None,
                scope: "project".to_string(),
                status: "active".to_string(),
                files: None,
                created_at_epoch: None,
                access_count: None,
                last_accessed_epoch: None,
            },
            GoldenMemory {
                project: "synthetic/provider-test".to_string(),
                topic_key: Some("provider-test-control".to_string()),
                title: "Provider control".to_string(),
                content: "Unrelated control memory about build logs.".to_string(),
                memory_type: "decision".to_string(),
                branch: None,
                scope: "project".to_string(),
                status: "active".to_string(),
                files: None,
                created_at_epoch: None,
                access_count: None,
                last_accessed_epoch: None,
            },
        ],
        queries: vec![GoldenQuery {
            id: "provider-comparison-test-en-01".to_string(),
            query: "owner mauve buffer restore".to_string(),
            category: "retrieval".to_string(),
            slice: Some(PROVIDER_COMPARISON_SLICE.to_string()),
            hop_path: None,
            project: Some("synthetic/provider-test".to_string()),
            branch: None,
            memory_type: None,
            relevant_ids: vec![],
            evidence_refs: vec![EvidenceRef {
                topic_key: Some("provider-test-target".to_string()),
                memory_type: Some("decision".to_string()),
                text_contains: Some("violet cache recovery".to_string()),
                ..EvidenceRef::default()
            }],
            expect_abstain: false,
            false_premise: false,
            notes: None,
        }],
    }
}

fn with_clean_provider_env<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    let _env_guard = crate::runtime_config::ENV_LOCK
        .lock()
        .map_err(|_| anyhow!("embedding provider env lock poisoned"))?;
    let temp_data_dir = std::env::temp_dir().join(format!(
        "remem-provider-comparison-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let keys = BASE_ENV_KEYS
        .iter()
        .map(|key| (*key).to_string())
        .chain(["REMEM_DATA_DIR".to_string()])
        .collect::<Vec<_>>();
    let saved = keys
        .iter()
        .map(|key| (key.clone(), std::env::var(key).ok()))
        .collect::<Vec<_>>();
    for key in &keys {
        unsafe { std::env::remove_var(key) };
    }
    unsafe { std::env::set_var("REMEM_DATA_DIR", &temp_data_dir) };
    let result = f();
    for (key, value) in saved {
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }
    let _ = fs::remove_dir_all(temp_data_dir);
    result
}
