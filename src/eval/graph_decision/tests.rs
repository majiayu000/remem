use super::*;

#[test]
fn graph_decision_eval_wires_literal_graph_after_material_gain() -> Result<()> {
    let report = run_graph_decision_eval(GraphDecisionEvalOptions::default())?;
    assert_eq!(report.decision, GraphDecision::WireLiteralGraphTraversal);
    assert_eq!(
        report.embedding_profile,
        GraphDecisionEmbeddingProfile {
            configured_provider: "feature-hash".to_string(),
            active_provider: "feature-hash".to_string(),
            fallback_provider: None,
            model_id: crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL.to_string(),
            dimensions: crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_DIMENSIONS,
            degraded: false,
            disabled: false,
        }
    );
    assert_eq!(
        report.evaluated_channel,
        EvaluatedGraphChannel::LiteralGraphEdges
    );
    assert!(report.graph_edges_evaluated);
    assert_eq!(
        report.graph_edges_retrieval_decision,
        GraphEdgesRetrievalDecision::WireProductionChannel
    );
    assert!(report.checks.all_checks_passed, "{report:#?}");
    assert!(report.checks.safe_to_wire_literal_graph);
    assert!(report.checks.benefit_threshold_met);
    assert!(report.checks.non_associative_zero_regression);
    assert!(report.checks.literal_two_hop_observed);
    assert!(report.checks.zero_scope_leak);
    assert!(report.deltas.associative_evidence_recall_at_k >= BENEFIT_THRESHOLD);
    let standard_non_associative = report
        .standard
        .non_associative_slices
        .metrics
        .as_ref()
        .context("standard non-associative metrics")?;
    let literal_non_associative = report
        .literal_graph
        .non_associative_slices
        .metrics
        .as_ref()
        .context("literal non-associative metrics")?;
    assert_eq!(
        literal_non_associative.precision_at_k,
        standard_non_associative.precision_at_k
    );
    assert!(non_associative_slices_not_lower(
        &report.standard.non_associative_by_slice,
        &report.literal_graph.non_associative_by_slice,
    ));
    let mut degraded = report.literal_graph.non_associative_by_slice.clone();
    let (slice, standard_slice) = report
        .standard
        .non_associative_by_slice
        .iter()
        .find(|(_, slice)| {
            slice
                .metrics
                .as_ref()
                .is_some_and(|metrics| metrics.hit_at_k > 0.0)
        })
        .context("non-associative scored slice")?;
    degraded
        .get_mut(slice)
        .and_then(|slice| slice.metrics.as_mut())
        .context("candidate non-associative scored slice")?
        .hit_at_k = standard_slice
        .metrics
        .as_ref()
        .context("standard slice metrics")?
        .hit_at_k
        - 0.25;
    assert!(!non_associative_slices_not_lower(
        &report.standard.non_associative_by_slice,
        &degraded,
    ));
    Ok(())
}

#[test]
fn graph_decision_eval_ignores_and_restores_ambient_local_provider() -> Result<()> {
    let _env_guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("graph decision test environment lock poisoned"))?;
    let keys = [
        "REMEM_CONFIG",
        "REMEM_EMBEDDINGS_PROVIDER",
        "REMEM_EMBEDDINGS_FALLBACK",
        "REMEM_EMBEDDINGS_MODEL_DIR",
    ];
    let saved = keys
        .iter()
        .map(|key| (*key, std::env::var_os(key)))
        .collect::<Vec<_>>();
    for key in keys {
        unsafe { std::env::remove_var(key) };
    }
    let missing_model_dir = std::env::temp_dir().join(format!(
        "remem-graph-decision-missing-model-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    unsafe {
        std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "local");
        std::env::set_var("REMEM_EMBEDDINGS_FALLBACK", "off");
        std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", &missing_model_dir);
    }

    let result = run_graph_decision_eval(GraphDecisionEvalOptions::default());
    let restored_provider = std::env::var("REMEM_EMBEDDINGS_PROVIDER").ok();
    let restored_fallback = std::env::var("REMEM_EMBEDDINGS_FALLBACK").ok();
    let restored_model_dir = std::env::var_os("REMEM_EMBEDDINGS_MODEL_DIR");
    for (key, value) in saved {
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    let report = result?;
    assert_eq!(report.embedding_profile.active_provider, "feature-hash");
    assert_eq!(restored_provider.as_deref(), Some("local"));
    assert_eq!(restored_fallback.as_deref(), Some("off"));
    assert_eq!(
        restored_model_dir.as_deref(),
        Some(missing_model_dir.as_os_str())
    );
    Ok(())
}

#[test]
fn graph_decision_eval_rejects_dataset_without_associative_slice() -> Result<()> {
    let mut dataset = golden::load_dataset(DEFAULT_DATASET_PATH)?;
    for query in &mut dataset.queries {
        if query.slice_label() == "associative" {
            query.slice = Some("paraphrase".to_string());
        }
    }

    let error = run_graph_decision_dataset(
        dataset,
        DEFAULT_DATASET_PATH.to_string(),
        GraphDecisionEvalOptions::default().k,
    )
    .expect_err("dataset without associative slice must fail the graph decision gate");

    assert!(error
        .to_string()
        .contains("requires scored associative queries"));
    Ok(())
}
