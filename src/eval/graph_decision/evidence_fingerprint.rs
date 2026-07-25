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
    "src/retrieval/graph.rs",
    "src/retrieval/graph/query.rs",
    "src/retrieval/graph/traverse.rs",
    "src/retrieval/graph/types.rs",
    "src/retrieval/search/memory/text/graph.rs",
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
/// different dataset; implementation paths are fixed by `IMPLEMENTATION_INPUTS`.
pub fn compute(dataset_path: &str) -> Result<GraphEvidenceFingerprint> {
    let mut raw: Vec<(String, &'static str, Vec<u8>)> =
        Vec::with_capacity(1 + IMPLEMENTATION_INPUTS.len());
    raw.push((
        dataset_path.to_string(),
        "dataset",
        read_bytes(dataset_path)?,
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
