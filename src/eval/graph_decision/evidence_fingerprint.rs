//! Content fingerprints for the graph-decision eval evidence (GH900 / GH853).
//!
//! `eval/graph-decision/report.json` used to carry metrics plus a date string
//! only, so a stale report could survive source or dataset changes silently.
//! This module computes a deterministic, length-prefixed SHA-256 fingerprint
//! over the golden dataset and over every evaluator/retrieval source file that
//! can affect the graph-decision result. The guard test below recomputes the
//! fingerprint from the live tree and fails loudly when the committed report is
//! stale, mirroring the associative baseline guard in `src/eval/associative.rs`.

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Fingerprint algorithm identifier, embedded in the report so a future format
/// change is explicit instead of silently reinterpreting old digests.
pub const ALGORITHM: &str = "sha256-length-prefixed-v1";

/// Evaluator and retrieval source files that can change the graph-decision
/// result (decision + metrics). Kept sorted; the dataset is added separately
/// because its path is parameterized. Any new file that can affect the decision
/// must be listed here so the fingerprint coverage stays auditable.
///
/// This fingerprint module is deliberately NOT listed: it does not affect the
/// graph-decision result, and hashing itself would be circular (any edit here,
/// even test-only, would force a report regeneration). Fingerprint-logic drift
/// is still caught because a changed `compute` yields different digests than the
/// committed report.
const IMPLEMENTATION_INPUTS: &[&str] = &[
    "src/eval/golden.rs",
    "src/eval/golden/run.rs",
    "src/eval/golden/types.rs",
    "src/eval/graph_decision.rs",
    "src/eval/provider_comparison.rs",
    "src/memory.rs",
    "src/memory/facts.rs",
    "src/memory/graph_contract.rs",
    "src/memory/graph_provenance.rs",
    "src/memory/lifecycle.rs",
    "src/memory/operation.rs",
    "src/memory/promote.rs",
    "src/memory/promote/slug.rs",
    "src/memory/retrieval_enrichment.rs",
    "src/memory/search_context.rs",
    "src/memory/semantic_dedup.rs",
    "src/memory/staleness.rs",
    "src/memory/staleness/capabilities.rs",
    "src/memory/staleness/path.rs",
    "src/memory/staleness/util.rs",
    "src/memory/state_key.rs",
    "src/memory/store.rs",
    "src/memory/store/read.rs",
    "src/memory/store/write.rs",
    "src/memory/suppression.rs",
    "src/memory/types.rs",
    "src/migrate.rs",
    "src/migrate/content_identity.rs",
    "src/migrate/run.rs",
    "src/migrate/schema_drift.rs",
    "src/migrate/schema_drift/exists.rs",
    "src/migrate/schema_drift/invariants.rs",
    "src/migrate/schema_drift/invariants/v068.rs",
    "src/migrate/schema_drift/invariants/v070.rs",
    "src/migrate/schema_drift/invariants/v071.rs",
    "src/migrate/schema_drift/invariants/v072.rs",
    "src/migrate/schema_drift/invariants/v073.rs",
    "src/migrate/schema_drift/invariants/v076.rs",
    "src/migrate/schema_drift/invariants/v076/shape.rs",
    "src/migrate/schema_drift/invariants/v077.rs",
    "src/migrate/schema_drift/invariants/v078.rs",
    "src/migrate/schema_drift/invariants/v079.rs",
    "src/migrate/schema_drift/invariants/v080.rs",
    "src/migrate/schema_drift/invariants/v081.rs",
    "src/migrate/schema_drift/invariants/v082.rs",
    "src/migrate/schema_drift/invariants/v083.rs",
    "src/migrate/schema_drift/invariants/v084.rs",
    "src/migrate/schema_drift/invariants/v084/shape.rs",
    "src/migrate/schema_drift/invariants/v085.rs",
    "src/migrate/schema_drift/invariants/v086.rs",
    "src/migrate/schema_drift/invariants/v087.rs",
    "src/migrate/state.rs",
    "src/migrate/transition.rs",
    "src/migrate/types.rs",
    "src/project_id.rs",
    "src/retrieval/embedding.rs",
    "src/retrieval/embedding/config.rs",
    "src/retrieval/embedding/fallback.rs",
    "src/retrieval/embedding/index_text.rs",
    "src/retrieval/embedding/network_policy.rs",
    "src/retrieval/embedding/status.rs",
    "src/retrieval/entity.rs",
    "src/retrieval/entity/extract.rs",
    "src/retrieval/entity/link.rs",
    "src/retrieval/entity/search.rs",
    "src/retrieval/entity/search/lookup.rs",
    "src/retrieval/entity/search/runner.rs",
    "src/retrieval/entity/search/sql.rs",
    "src/retrieval/graph.rs",
    "src/retrieval/graph/query.rs",
    "src/retrieval/graph/traverse.rs",
    "src/retrieval/graph/types.rs",
    "src/retrieval/memory_search.rs",
    "src/retrieval/memory_search/filters.rs",
    "src/retrieval/memory_search/fts.rs",
    "src/retrieval/memory_search/like.rs",
    "src/retrieval/query_expand.rs",
    "src/retrieval/query_expand/expand.rs",
    "src/retrieval/query_expand/tokenize.rs",
    "src/retrieval/query_expand/translations.rs",
    "src/retrieval/rerank.rs",
    "src/retrieval/rerank/config.rs",
    "src/retrieval/rerank/inventory.rs",
    "src/retrieval/rerank/model.rs",
    "src/retrieval/rerank/stage.rs",
    "src/retrieval/rerank/types.rs",
    "src/retrieval/search.rs",
    "src/retrieval/search/common.rs",
    "src/retrieval/search/memory.rs",
    "src/retrieval/search/memory/claim.rs",
    "src/retrieval/search/memory/claim/cjk_relational.rs",
    "src/retrieval/search/memory/claim/query_scaffold.rs",
    "src/retrieval/search/memory/runner.rs",
    "src/retrieval/search/memory/source_anchor.rs",
    "src/retrieval/search/memory/suppression_filter.rs",
    "src/retrieval/search/memory/text.rs",
    "src/retrieval/search/memory/text/explain_build.rs",
    "src/retrieval/search/memory/text/format.rs",
    "src/retrieval/search/memory/text/graph.rs",
    "src/retrieval/search/memory/text/support.rs",
    "src/retrieval/search/memory/text/support/fact.rs",
    "src/retrieval/search/memory/text/support/graph_claim.rs",
    "src/retrieval/search/memory/usage_rank.rs",
    "src/retrieval/search/memory/weights.rs",
    "src/retrieval/search_multihop.rs",
    "src/retrieval/search_multihop/discover.rs",
    "src/retrieval/search_multihop/expand.rs",
    "src/retrieval/search_multihop/merge.rs",
    "src/retrieval/search_multihop/search.rs",
    "src/retrieval/search_multihop/types.rs",
    "src/retrieval/temporal.rs",
    "src/retrieval/temporal/fact_keys.rs",
    "src/retrieval/temporal/fact_labels.rs",
    "src/retrieval/temporal/parse.rs",
    "src/retrieval/temporal/parse/boundary.rs",
    "src/retrieval/temporal/search.rs",
    "src/retrieval/temporal/types.rs",
    "src/retrieval/vector.rs",
    "src/retrieval/vector_candidates.rs",
    "src/runtime_config.rs",
];

/// Synthetic input name for the exact SQL bundle applied by
/// `crate::migrate::run_migrations` in every graph-decision arm. The registry
/// source itself is fingerprinted above; this bundle additionally binds the
/// bytes behind every `include_str!` entry instead of only the registration
/// statements.
const MIGRATION_BUNDLE_PATH: &str = "crate::migrate::MIGRATIONS";

/// Kept in registry order. The structural test below compares every path,
/// version, name, and SQL byte with the test-visible runtime `MIGRATIONS`
/// registry, so adding, removing, reordering, or retargeting a migration cannot
/// leave this bundle stale silently.
const MIGRATION_SQL_INPUTS: &[&str] = &[
    "src/migrations/v001_baseline.sql",
    "src/migrations/v002_raw_messages.sql",
    "src/migrations/v003_host_identity.sql",
    "src/migrations/v004_worker_heartbeat.sql",
    "src/migrations/v005_memories_fts_active_filter.sql",
    "src/migrations/v006_capture_pipeline.sql",
    "src/migrations/v007_session_rollup_ranges.sql",
    "src/migrations/v008_observation_evidence.sql",
    "src/migrations/v009_memory_candidate_promotion.sql",
    "src/migrations/v010_ai_usage_token_breakdown.sql",
    "src/migrations/v011_reprice_ai_usage_events.sql",
    "src/migrations/v012_memory_search_context.sql",
    "src/migrations/v013_memory_temporal_facts.sql",
    "src/migrations/v014_procedure_verifications.sql",
    "src/migrations/v015_rebuild_memory_search_context.sql",
    "src/migrations/v016_context_injection_gate.sql",
    "src/migrations/v017_memory_lessons.sql",
    "src/migrations/v018_commit_session_links.sql",
    "src/migrations/v019_memory_ownership.sql",
    "src/migrations/v020_memory_fts_all_status.sql",
    "src/migrations/v021_raw_messages_session_dedup.sql",
    "src/migrations/v022_memory_state_keys.sql",
    "src/migrations/v023_topic_segments.sql",
    "src/migrations/v024_memory_operation_log.sql",
    "src/migrations/v025_memory_edges.sql",
    "src/migrations/v026_memory_claims.sql",
    "src/migrations/v027_compressed_observation_sources.sql",
    "src/migrations/v028_raw_ingest_failures.sql",
    "src/migrations/v029_memory_embeddings.sql",
    "src/migrations/v030_dream_cluster_decisions.sql",
    "src/migrations/v031_graph_edges.sql",
    "src/migrations/v032_candidate_block_reason.sql",
    "src/migrations/v033_graph_candidates.sql",
    "src/migrations/v034_graph_edge_file_nodes.sql",
    "src/migrations/v035_context_injection_data_version.sql",
    "src/migrations/v036_capture_drop_events.sql",
    "src/migrations/v037_graph_edge_source_candidate_integrity.sql",
    "src/migrations/v038_extraction_replay_ranges.sql",
    "src/migrations/v039_context_injection_items.sql",
    "src/migrations/v040_memory_fact_invalidations.sql",
    "src/migrations/v041_content_identity_sha256.sql",
    "src/migrations/v042_reference_time_epoch.sql",
    "src/migrations/v043_graph_candidate_prompt_memory_refs.sql",
    "src/migrations/v044_memory_embeddings_profile_index.sql",
    "src/migrations/v045_memory_usage_columns.sql",
    "src/migrations/v046_ai_usage_session_id.sql",
    "src/migrations/v047_lesson_outcome_metadata.sql",
    "src/migrations/v048_failure_lesson_feed_events.sql",
    "src/migrations/v049_user_context_claims.sql",
    "src/migrations/v050_user_context_summaries.sql",
    "src/migrations/v051_memory_suppressions_feedback.sql",
    "src/migrations/v052_user_context_candidates.sql",
    "src/migrations/v053_workstream_identity_continuity.sql",
    "src/migrations/v054_memory_candidate_source_kind.sql",
    "src/migrations/v055_session_ingest_cursors.sql",
    "src/migrations/v056_raw_messages_source_root_key.sql",
    "src/migrations/v057_failure_lifecycle.sql",
    "src/migrations/v058_memory_embeddings_multimodel_key.sql",
    "src/migrations/v059_candidate_review_metadata.sql",
    "src/migrations/v060_memory_poisoning_defense.sql",
    "src/migrations/v061_memory_poisoning_injection_drops.sql",
    "src/migrations/v062_preference_rule_state.sql",
    "src/migrations/v063_procedure_exports.sql",
    "src/migrations/v064_reject_legacy_summary_jobs.sql",
    "src/migrations/v065_preference_reinforcement.sql",
    "src/migrations/v066_session_rollup_evidence_checkpoint.sql",
    "src/migrations/v067_capture_git_evidence.sql",
    "src/migrations/v068_session_rollup_followup_checkpoint.sql",
    "src/migrations/v069_job_queue_atomicity.sql",
    "src/migrations/v070_web_console_governance.sql",
    "src/migrations/v071_raw_session_identity.sql",
    "src/migrations/v072_memory_retrieval_enrichment.sql",
    "src/migrations/v073_session_summary_poisoning.sql",
    "src/migrations/v074_git_commit_staleness_index.sql",
    "src/migrations/v075_automatic_cleanup.sql",
    "src/migrations/v076_dream_poisoning_quarantine.sql",
    "src/migrations/v077_dream_backfill.sql",
    "src/migrations/v078_event_capture_projection.sql",
    "src/migrations/v079_candidate_spo_facts.sql",
    "src/migrations/v080_candidate_outcome.sql",
    "src/migrations/v081_context_bundle_audits.sql",
    "src/migrations/v082_project_identity_aliases.sql",
    "src/migrations/v083_retrieval_enrichment_budget.sql",
    "src/migrations/v084_session_observatory.sql",
    "src/migrations/v085_legacy_pending_bridge_state.sql",
    "src/migrations/v086_memory_activation_boundary.sql",
    "src/migrations/v087_activation_result_trust.sql",
    "src/migrations/v088_activation_legacy_trust.sql",
];

#[derive(Debug, Clone, Serialize)]
pub struct GraphEvidenceFingerprint {
    pub algorithm: String,
    pub dataset_sha256: String,
    pub implementation_sha256: String,
    pub combined_sha256: String,
    pub inputs: Vec<GraphEvidenceFingerprintInput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEvidenceFingerprintInput {
    pub path: String,
    pub role: String,
    pub byte_len: u64,
    pub sha256: String,
}

/// Compute the dataset + implementation fingerprints by reading the live files
/// from disk. `dataset_path` is parameterized so callers/tests can point at a
/// different dataset; implementation paths are fixed by `IMPLEMENTATION_INPUTS`
/// plus the structurally verified migration SQL bundle.
pub fn compute(dataset_path: &str) -> Result<GraphEvidenceFingerprint> {
    let mut raw: Vec<(String, &'static str, Vec<u8>)> =
        Vec::with_capacity(2 + IMPLEMENTATION_INPUTS.len());
    raw.push((
        dataset_path.to_string(),
        "dataset",
        read_bytes(dataset_path)?,
    ));
    raw.push((
        MIGRATION_BUNDLE_PATH.to_string(),
        "implementation",
        migration_bundle_bytes()?,
    ));
    for path in IMPLEMENTATION_INPUTS {
        raw.push(((*path).to_string(), "implementation", read_bytes(path)?));
    }
    raw.sort_by(|left, right| left.0.cmp(&right.0));

    let mut dataset_hasher = Sha256::new();
    let mut implementation_hasher = Sha256::new();
    let mut combined_hasher = Sha256::new();
    let mut inputs = Vec::with_capacity(raw.len());
    for (path, role, bytes) in &raw {
        if *role == "dataset" {
            feed_length_prefixed(&mut dataset_hasher, path, bytes);
        } else {
            feed_length_prefixed(&mut implementation_hasher, path, bytes);
        }
        feed_length_prefixed(&mut combined_hasher, path, bytes);
        inputs.push(GraphEvidenceFingerprintInput {
            path: path.clone(),
            role: (*role).to_string(),
            byte_len: bytes.len() as u64,
            sha256: length_prefixed_sha256(path, bytes),
        });
    }

    Ok(GraphEvidenceFingerprint {
        algorithm: ALGORITHM.to_string(),
        dataset_sha256: hex_digest(&dataset_hasher.finalize()),
        implementation_sha256: hex_digest(&implementation_hasher.finalize()),
        combined_sha256: hex_digest(&combined_hasher.finalize()),
        inputs,
    })
}

fn read_bytes(path: &str) -> Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("read graph-decision fingerprint input {path}"))
}

fn migration_bundle_bytes() -> Result<Vec<u8>> {
    let mut bundle = Vec::new();
    bundle.extend_from_slice(&(MIGRATION_SQL_INPUTS.len() as u64).to_be_bytes());
    for path in MIGRATION_SQL_INPUTS {
        let sql = read_bytes(path)?;
        bundle.extend_from_slice(&(path.len() as u64).to_be_bytes());
        bundle.extend_from_slice(path.as_bytes());
        bundle.extend_from_slice(&(sql.len() as u64).to_be_bytes());
        bundle.extend_from_slice(&sql);
    }
    Ok(bundle)
}

/// Length-prefixed encoding prevents boundary ambiguity between (path, content)
/// pairs: `len(path) || path || len(content) || content`.
fn feed_length_prefixed(hasher: &mut Sha256, path: &str, bytes: &[u8]) {
    hasher.update((path.len() as u64).to_be_bytes());
    hasher.update(path.as_bytes());
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn length_prefixed_sha256(path: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    feed_length_prefixed(&mut hasher, path, bytes);
    hex_digest(&hasher.finalize())
}

fn hex_digest(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::graph_decision::{
        run_graph_decision_eval, GraphDecisionEvalOptions, DEFAULT_DATASET_PATH,
        DEFAULT_REPORT_PATH,
    };

    #[test]
    fn fingerprint_is_deterministic_with_sorted_input_list() -> Result<()> {
        let first = compute(DEFAULT_DATASET_PATH)?;
        let second = compute(DEFAULT_DATASET_PATH)?;
        assert_eq!(first.combined_sha256, second.combined_sha256);
        assert_eq!(first.dataset_sha256, second.dataset_sha256);
        assert_eq!(first.implementation_sha256, second.implementation_sha256);
        assert_eq!(first.algorithm, ALGORITHM);
        assert!(!first.combined_sha256.is_empty());
        assert!(first
            .inputs
            .iter()
            .any(|input| input.role == "dataset" && input.path == DEFAULT_DATASET_PATH));
        assert!(first
            .inputs
            .iter()
            .any(|input| input.role == "implementation"));
        let paths = first
            .inputs
            .iter()
            .map(|input| input.path.clone())
            .collect::<Vec<_>>();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "fingerprint input list must be sorted");
        Ok(())
    }

    #[test]
    fn fingerprint_changes_when_a_listed_dataset_input_changes() -> Result<()> {
        // Stale-report rejection mechanism, exercised without mutating the
        // working tree: a fingerprint over a byte-mutated copy of the dataset
        // must differ from the live fingerprint.
        let baseline = compute(DEFAULT_DATASET_PATH)?;
        let original = std::fs::read_to_string(DEFAULT_DATASET_PATH)?;
        let mutated = format!("{original}\n");
        let tmp = std::env::temp_dir().join("remem-gh900-fingerprint-mutation.json");
        std::fs::write(&tmp, mutated)?;
        let mutated_fingerprint = compute(tmp.to_str().context("temp path is valid UTF-8")?)?;
        assert_ne!(baseline.dataset_sha256, mutated_fingerprint.dataset_sha256);
        assert_ne!(
            baseline.combined_sha256,
            mutated_fingerprint.combined_sha256
        );
        assert_eq!(
            baseline.implementation_sha256, mutated_fingerprint.implementation_sha256,
            "dataset-only mutation must not change the implementation fingerprint"
        );
        Ok(())
    }

    #[test]
    fn fingerprint_covers_result_changing_retrieval_inputs() -> Result<()> {
        let fingerprint = compute(DEFAULT_DATASET_PATH)?;
        for required_path in [
            "src/eval/provider_comparison.rs",
            "src/memory.rs",
            "src/memory/facts.rs",
            "src/memory/retrieval_enrichment.rs",
            "src/memory/search_context.rs",
            "src/memory/staleness.rs",
            "src/memory/staleness/capabilities.rs",
            "src/memory/staleness/path.rs",
            "src/memory/staleness/util.rs",
            "src/memory/store.rs",
            "src/memory/store/read.rs",
            "src/memory/store/write.rs",
            "src/memory/suppression.rs",
            "src/memory/types.rs",
            "src/retrieval/embedding.rs",
            "src/retrieval/embedding/config.rs",
            "src/retrieval/embedding/fallback.rs",
            "src/retrieval/embedding/index_text.rs",
            "src/retrieval/embedding/network_policy.rs",
            "src/retrieval/embedding/status.rs",
            "src/retrieval/entity/search/lookup.rs",
            "src/retrieval/entity/search/sql.rs",
            "src/retrieval/memory_search.rs",
            "src/retrieval/memory_search/filters.rs",
            "src/retrieval/memory_search/fts.rs",
            "src/retrieval/memory_search/like.rs",
            "src/retrieval/query_expand.rs",
            "src/retrieval/query_expand/expand.rs",
            "src/retrieval/query_expand/tokenize.rs",
            "src/retrieval/query_expand/translations.rs",
            "src/retrieval/rerank.rs",
            "src/retrieval/rerank/config.rs",
            "src/retrieval/rerank/inventory.rs",
            "src/retrieval/rerank/model.rs",
            "src/retrieval/rerank/stage.rs",
            "src/retrieval/rerank/types.rs",
            "src/retrieval/search.rs",
            "src/retrieval/search/common.rs",
            "src/retrieval/search/memory.rs",
            "src/retrieval/search/memory/claim.rs",
            "src/retrieval/search/memory/claim/cjk_relational.rs",
            "src/retrieval/search/memory/claim/query_scaffold.rs",
            "src/retrieval/search/memory/runner.rs",
            "src/retrieval/search/memory/source_anchor.rs",
            "src/retrieval/search/memory/suppression_filter.rs",
            "src/retrieval/search/memory/text.rs",
            "src/retrieval/search/memory/text/explain_build.rs",
            "src/retrieval/search/memory/text/format.rs",
            "src/retrieval/search/memory/text/graph.rs",
            "src/retrieval/search/memory/text/support.rs",
            "src/retrieval/search/memory/text/support/fact.rs",
            "src/retrieval/search/memory/text/support/graph_claim.rs",
            "src/retrieval/search/memory/usage_rank.rs",
            "src/retrieval/search/memory/weights.rs",
            "src/retrieval/temporal.rs",
            "src/retrieval/temporal/fact_keys.rs",
            "src/retrieval/temporal/fact_labels.rs",
            "src/retrieval/temporal/parse.rs",
            "src/retrieval/temporal/parse/boundary.rs",
            "src/retrieval/temporal/search.rs",
            "src/retrieval/temporal/types.rs",
            "src/retrieval/vector.rs",
            "src/retrieval/vector_candidates.rs",
            "src/runtime_config.rs",
        ] {
            assert!(
                fingerprint
                    .inputs
                    .iter()
                    .any(|input| { input.role == "implementation" && input.path == required_path }),
                "missing graph-decision fingerprint input {required_path}"
            );
        }
        Ok(())
    }

    #[test]
    fn fingerprint_covers_every_graph_decision_arm_direct_dependency() -> Result<()> {
        let fingerprint = compute(DEFAULT_DATASET_PATH)?;
        for required_path in [
            "src/eval/golden/run.rs",
            "src/eval/graph_decision.rs",
            "src/memory/graph_contract.rs",
            "src/memory/graph_provenance.rs",
            "src/memory/lifecycle.rs",
            "src/memory/operation.rs",
            "src/memory/promote.rs",
            "src/memory/promote/slug.rs",
            "src/memory/semantic_dedup.rs",
            "src/memory/state_key.rs",
            "src/migrate.rs",
            "src/migrate/content_identity.rs",
            "src/migrate/run.rs",
            "src/migrate/schema_drift.rs",
            "src/migrate/schema_drift/exists.rs",
            "src/migrate/schema_drift/invariants.rs",
            "src/migrate/schema_drift/invariants/v068.rs",
            "src/migrate/schema_drift/invariants/v070.rs",
            "src/migrate/schema_drift/invariants/v071.rs",
            "src/migrate/schema_drift/invariants/v072.rs",
            "src/migrate/schema_drift/invariants/v073.rs",
            "src/migrate/schema_drift/invariants/v076.rs",
            "src/migrate/schema_drift/invariants/v077.rs",
            "src/migrate/schema_drift/invariants/v078.rs",
            "src/migrate/schema_drift/invariants/v079.rs",
            "src/migrate/schema_drift/invariants/v080.rs",
            "src/migrate/schema_drift/invariants/v081.rs",
            "src/migrate/schema_drift/invariants/v084.rs",
            "src/migrate/state.rs",
            "src/migrate/transition.rs",
            "src/migrate/types.rs",
            "src/project_id.rs",
            "src/retrieval/entity/link.rs",
            "src/retrieval/search_multihop.rs",
            "src/retrieval/search_multihop/discover.rs",
            "src/retrieval/search_multihop/expand.rs",
            "src/retrieval/search_multihop/merge.rs",
            "src/retrieval/search_multihop/search.rs",
            "src/retrieval/search_multihop/types.rs",
            MIGRATION_BUNDLE_PATH,
        ] {
            assert!(
                fingerprint
                    .inputs
                    .iter()
                    .any(|input| input.role == "implementation" && input.path == required_path),
                "missing graph-decision direct dependency {required_path}"
            );
        }
        Ok(())
    }

    #[test]
    fn migration_bundle_matches_runtime_registry_structurally() -> Result<()> {
        let migrations = crate::migrate::MIGRATIONS;
        assert_eq!(
            MIGRATION_SQL_INPUTS.len(),
            migrations.len(),
            "migration SQL bundle must cover every runtime registry entry"
        );

        let mut expected_bundle = Vec::new();
        expected_bundle.extend_from_slice(&(migrations.len() as u64).to_be_bytes());
        for (path, migration) in MIGRATION_SQL_INPUTS.iter().zip(migrations) {
            let expected_path = format!(
                "src/migrations/v{:03}_{}.sql",
                migration.version, migration.name
            );
            assert_eq!(
                *path, expected_path,
                "migration bundle path/order drifted from runtime registry"
            );
            let live_sql = read_bytes(path)?;
            assert_eq!(
                live_sql.as_slice(),
                migration.sql.as_bytes(),
                "migration bundle SQL bytes drifted from runtime registry for {path}"
            );
            expected_bundle.extend_from_slice(&(path.len() as u64).to_be_bytes());
            expected_bundle.extend_from_slice(path.as_bytes());
            expected_bundle.extend_from_slice(&(migration.sql.len() as u64).to_be_bytes());
            expected_bundle.extend_from_slice(migration.sql.as_bytes());
        }

        let bundle = migration_bundle_bytes()?;
        assert_eq!(bundle, expected_bundle);
        let fingerprint = compute(DEFAULT_DATASET_PATH)?;
        let bundle_input = fingerprint
            .inputs
            .iter()
            .find(|input| input.path == MIGRATION_BUNDLE_PATH)
            .context("migration bundle fingerprint input")?;
        assert_eq!(bundle_input.role, "implementation");
        assert_eq!(bundle_input.byte_len, bundle.len() as u64);
        assert_eq!(
            bundle_input.sha256,
            length_prefixed_sha256(MIGRATION_BUNDLE_PATH, &bundle)
        );
        Ok(())
    }

    #[test]
    fn checked_in_graph_decision_report_matches_generated_fingerprint() -> Result<()> {
        // Mirror of the associative baseline guard: regenerate the report from
        // the live source/data and require the committed JSON to match. A stale
        // report (source or dataset changed without regeneration) fails loudly.
        let report = run_graph_decision_eval(GraphDecisionEvalOptions::default())?;
        let committed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(DEFAULT_REPORT_PATH)?)?;

        assert_eq!(
            committed["evidence_fingerprint"],
            serde_json::to_value(&report.evidence_fingerprint)?,
            "checked-in graph-decision report fingerprint is stale; regenerate eval/graph-decision/report.json"
        );
        assert_eq!(committed["version"], report.version);
        assert_eq!(committed["dataset_path"], report.dataset_path);
        assert_eq!(
            committed["embedding_profile"],
            serde_json::to_value(&report.embedding_profile)?
        );
        assert_eq!(
            committed["decision"],
            serde_json::to_value(report.decision)?
        );
        assert_eq!(committed["checks"], serde_json::to_value(&report.checks)?);
        // Compare only the deterministic metric deltas. `deltas.p95_latency_ms`
        // is intentionally excluded because retrieval latency varies per run;
        // the fingerprint above is the stale-report guard for source/data drift.
        let committed_deltas = &committed["deltas"];
        assert_eq!(
            committed_deltas["associative_recall_at_k"],
            serde_json::to_value(report.deltas.associative_recall_at_k)?
        );
        assert_eq!(
            committed_deltas["associative_evidence_recall_at_k"],
            serde_json::to_value(report.deltas.associative_evidence_recall_at_k)?
        );
        assert_eq!(
            committed_deltas["associative_ndcg_at_10"],
            serde_json::to_value(report.deltas.associative_ndcg_at_10)?
        );
        assert_eq!(
            committed_deltas["non_associative_recall_at_k"],
            serde_json::to_value(report.deltas.non_associative_recall_at_k)?
        );
        assert_eq!(
            committed_deltas["non_associative_evidence_recall_at_k"],
            serde_json::to_value(report.deltas.non_associative_evidence_recall_at_k)?
        );
        assert_eq!(
            committed_deltas["non_associative_ndcg_at_10"],
            serde_json::to_value(report.deltas.non_associative_ndcg_at_10)?
        );
        Ok(())
    }
}
