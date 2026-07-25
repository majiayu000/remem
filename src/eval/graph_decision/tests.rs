use super::*;

#[test]
fn graph_decision_eval_wires_literal_graph_after_material_gain() -> Result<()> {
    let report = run_graph_decision_eval(GraphDecisionEvalOptions::default())?;
    assert_eq!(report.decision, GraphDecision::WireLiteralGraphTraversal);
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
