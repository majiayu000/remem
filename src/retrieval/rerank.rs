//! Optional second-stage local cross-encoder rerank (GH-851).
//!
//! Default-off. Both standard curated-memory search and the SessionStart
//! implicit query share one final-candidate rerank stage: same query
//! boundary, same document projection, same top-N/top-k, same failure
//! fallback, same `disabled_reason` set, and same timing phases. Query paths
//! never download models; installation is the explicit
//! `remem reranker download` action only.

pub mod config;
pub mod inventory;
mod model;
mod stage;
#[cfg(test)]
mod tests;
pub mod types;

use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::memory::{self, Memory};

use config::{resolve_rerank_config, RerankConfig};
use inventory::{inventory_state, RerankerInventoryState, RerankerPreset};
use types::{RerankCandidate, RerankOutcome};

pub use types::{RerankDisabledReason, RerankExplain};

/// Canonical bounded query-document projection shared by every calling path.
/// Only human-approved local fields (title, type, content) enter the model;
/// authorization metadata (project, owner, scope) never does.
fn candidate_document(memory: &Memory) -> String {
    format!(
        "title: {}\ntype: {}\ncontent: {}",
        memory.title, memory.memory_type, memory.text
    )
}

fn build_candidates(
    ordered: &[Memory],
    verify_before_trust_ids: &HashSet<i64>,
) -> Vec<RerankCandidate> {
    ordered
        .iter()
        .enumerate()
        .map(|(baseline_rank, memory)| RerankCandidate {
            id: memory.id,
            baseline_rank,
            verify_before_trust: verify_before_trust_ids.contains(&memory.id),
            document: candidate_document(memory),
        })
        .collect()
}

fn reorder_applied(ordered: Vec<Memory>, outcome: &RerankOutcome) -> Vec<Memory> {
    if !outcome.applied() {
        return ordered;
    }
    let mut by_id: std::collections::HashMap<i64, Memory> = ordered
        .into_iter()
        .map(|memory| (memory.id, memory))
        .collect();
    outcome
        .ordered_ids
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect()
}

/// Standard-search entry: applies the shared stage to the eligible,
/// source-anchor-demoted baseline order. `verify-before-trust` labels are
/// looked up only for the memories already in the baseline.
pub(crate) fn apply_to_search(
    conn: &Connection,
    query: &str,
    ordered: Vec<Memory>,
) -> Result<(Vec<Memory>, RerankOutcome)> {
    let config = resolve_rerank_config()?;
    if !config.enabled {
        return Ok((
            ordered,
            RerankOutcome::not_applied(RerankDisabledReason::Off),
        ));
    }
    let verify_before_trust_ids = verify_before_trust_ids(conn, &ordered);
    apply_with_config(&config, query, ordered, &verify_before_trust_ids)
}

/// SessionStart entry: the caller passes its own final baseline order plus
/// the `verify-before-trust` ids from its already loaded staleness labels.
/// The order is only mutated on a successful `Applied` outcome; on any error
/// or `NotApplied` status the complete baseline order is left untouched.
pub(crate) fn apply_with_vbt(
    query: Option<&str>,
    ordered: &mut Vec<Memory>,
    verify_before_trust_ids: &HashSet<i64>,
) -> Result<RerankOutcome> {
    let config = resolve_rerank_config()?;
    if !config.enabled {
        return Ok(RerankOutcome::not_applied(RerankDisabledReason::Off));
    }
    let candidates = build_candidates(ordered, verify_before_trust_ids);
    let outcome = stage::run_stage(&config, query.unwrap_or(""), &candidates);
    if outcome.applied() {
        let baseline = std::mem::take(ordered);
        *ordered = reorder_applied(baseline, &outcome);
    }
    Ok(outcome)
}

fn apply_with_config(
    config: &RerankConfig,
    query: &str,
    ordered: Vec<Memory>,
    verify_before_trust_ids: &HashSet<i64>,
) -> Result<(Vec<Memory>, RerankOutcome)> {
    let candidates = build_candidates(&ordered, verify_before_trust_ids);
    let outcome = stage::run_stage(config, query, &candidates);
    let ordered = reorder_applied(ordered, &outcome);
    Ok((ordered, outcome))
}

fn verify_before_trust_ids(conn: &Connection, ordered: &[Memory]) -> HashSet<i64> {
    let now_epoch = chrono::Utc::now().timestamp();
    let labels = memory::staleness::memory_staleness_labels_for_memories_lossy(
        conn,
        ordered,
        now_epoch,
        |id, error| {
            crate::log::warn(
                "rerank",
                &format!("rerank source-anchor label fallback for memory {id}: {error}"),
            );
        },
    )
    .unwrap_or_default();
    labels
        .into_iter()
        .filter(|(_, label)| label.source_anchor == "verify-before-trust")
        .map(|(id, _)| id)
        .collect()
}

/// Structured reranker status shared by CLI status and doctor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RerankerStatusReport {
    pub enabled: bool,
    pub preset: String,
    pub model_id: String,
    pub upstream_model: String,
    pub model_root: String,
    pub install_dir: String,
    /// One of: `off`, `ready`, `missing`, `corrupt`.
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest_sha256: Option<String>,
    pub top_n: usize,
    pub top_k: usize,
}

pub fn reranker_status() -> Result<RerankerStatusReport> {
    let config = resolve_rerank_config()?;
    let preset = RerankerPreset::parse(&config.preset)?;
    let install_dir = inventory::install_dir_for_preset(&config, preset);
    let mut report = RerankerStatusReport {
        enabled: config.enabled,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        model_root: inventory::model_root(&config).display().to_string(),
        install_dir: install_dir.display().to_string(),
        state: String::new(),
        disabled_reason: None,
        detail: None,
        manifest_sha256: None,
        top_n: config.top_n,
        top_k: config.top_k,
    };
    let state = inventory_state(&config)?;
    match state {
        RerankerInventoryState::Ready(verified) => {
            report.state = "ready".to_string();
            report.manifest_sha256 = Some(verified.manifest_sha256);
        }
        RerankerInventoryState::Missing(detail) => {
            report.state = "missing".to_string();
            report.disabled_reason = Some(RerankDisabledReason::ModelMissing.as_str().to_string());
            report.detail = Some(detail);
        }
        RerankerInventoryState::Corrupt(detail) => {
            report.state = "corrupt".to_string();
            report.disabled_reason = Some(RerankDisabledReason::ModelCorrupt.as_str().to_string());
            report.detail = Some(detail);
        }
    }
    if !config.enabled {
        // Explicit off is not a failure: doctor reports OK with the stable
        // `off` reason regardless of on-disk inventory.
        report.state = "off".to_string();
        report.disabled_reason = Some(RerankDisabledReason::Off.as_str().to_string());
    }
    Ok(report)
}

pub fn download_reranker_model(model: Option<&str>) -> Result<inventory::RerankerDownloadReport> {
    inventory::download_model(model)
}
