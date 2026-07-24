//! Local reranker model loading and scoring.
//!
//! The runtime only ever builds a model from an already verified inventory
//! (`VerifiedRerankerModel`) via fastembed's user-defined path, which reads
//! local files and never touches a model hub. Network access is confined to
//! the explicit `remem reranker download` action in `inventory.rs`.

#[cfg(not(feature = "local-onnx"))]
use std::time::Instant;

#[cfg(not(feature = "local-onnx"))]
use super::inventory::VerifiedRerankerModel;

/// Failure classes of the local scoring path, mapped 1:1 onto the closed
/// `RerankDisabledReason` set by the stage orchestrator.
#[derive(Debug)]
pub(super) enum RerankModelError {
    Load(anyhow::Error),
    Inference(anyhow::Error),
    DeadlineExceeded,
}

pub(super) struct ScoreReport {
    /// One finite score per input document, in input order.
    pub scores: Vec<f32>,
    /// Cold model-load duration; `None` for warm (cached) requests so cold
    /// starts are never amortized into warm timing samples.
    pub load_ms: Option<u64>,
    pub inference_ms: u64,
}

/// Bounded batch size for deadline checks between inference batches.
const SCORE_BATCH_SIZE: usize = 8;

#[cfg(feature = "local-onnx")]
mod runtime {
    use std::cell::RefCell;
    use std::collections::{hash_map::Entry, HashMap};
    use std::path::PathBuf;
    use std::time::Instant;

    use anyhow::{Context, Result};

    use super::super::inventory::{role_path, VerifiedRerankerModel};
    use super::{RerankModelError, ScoreReport, SCORE_BATCH_SIZE};

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct RerankModelCacheKey {
        manifest_sha256: String,
        install_dir: PathBuf,
    }

    // Per-thread cache keyed by manifest hash: a manifest upgrade atomically
    // publishes a new hash, so new requests load the new version while
    // requests already holding the old read-only instance keep using it.
    // Failed initialization inserts nothing, so no half-initialized instance
    // is ever observable (mirrors the local embedding cache pattern).
    thread_local! {
        static RERANK_MODEL_CACHE: RefCell<HashMap<RerankModelCacheKey, fastembed::TextRerank>> =
            RefCell::new(HashMap::new());
    }

    fn build_model(verified: &VerifiedRerankerModel) -> Result<fastembed::TextRerank> {
        let roles = &verified.manifest.roles;
        let read_role = |relative: &str| -> Result<Vec<u8>> {
            let path = role_path(&verified.install_dir, relative)?;
            std::fs::read(&path).with_context(|| format!("read {}", path.display()))
        };
        let tokenizer_files = fastembed::TokenizerFiles {
            tokenizer_file: read_role(&roles.tokenizer_file)?,
            config_file: read_role(&roles.config_file)?,
            special_tokens_map_file: read_role(&roles.special_tokens_map_file)?,
            tokenizer_config_file: read_role(&roles.tokenizer_config_file)?,
        };
        let onnx_path = role_path(&verified.install_dir, &roles.onnx_file)?;
        let model = fastembed::UserDefinedRerankingModel::new(
            fastembed::OnnxSource::File(onnx_path),
            tokenizer_files,
        );
        fastembed::TextRerank::try_new_from_user_defined(
            model,
            fastembed::RerankInitOptionsUserDefined::default(),
        )
        .with_context(|| {
            format!(
                "initialize local reranker model {}",
                verified.manifest.model_id
            )
        })
    }

    pub(in super::super) fn score_documents(
        verified: &VerifiedRerankerModel,
        query: &str,
        documents: &[String],
        deadline: Instant,
    ) -> Result<ScoreReport, RerankModelError> {
        let key = RerankModelCacheKey {
            manifest_sha256: verified.manifest_sha256.clone(),
            install_dir: verified.install_dir.clone(),
        };
        RERANK_MODEL_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            let mut load_ms = None;
            let model = match cache.entry(key) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    if Instant::now() >= deadline {
                        return Err(RerankModelError::DeadlineExceeded);
                    }
                    let load_start = Instant::now();
                    let model = build_model(verified).map_err(RerankModelError::Load)?;
                    load_ms = Some(load_start.elapsed().as_millis() as u64);
                    entry.insert(model)
                }
            };
            let inference_start = Instant::now();
            // Request-private score buffer: nothing is shared across requests
            // and partial scores are discarded on any failure below.
            let mut scores = vec![0.0_f32; documents.len()];
            for (batch_index, batch) in documents.chunks(SCORE_BATCH_SIZE).enumerate() {
                if Instant::now() >= deadline {
                    return Err(RerankModelError::DeadlineExceeded);
                }
                let batch_refs: Vec<&str> = batch.iter().map(String::as_str).collect();
                let results = model
                    .rerank(query, batch_refs.as_slice(), false, Some(batch.len()))
                    .map_err(RerankModelError::Inference)?;
                if results.len() != batch.len() {
                    return Err(RerankModelError::Inference(anyhow::anyhow!(
                        "reranker returned {} scores for {} documents",
                        results.len(),
                        batch.len()
                    )));
                }
                for result in results {
                    if !result.score.is_finite() {
                        return Err(RerankModelError::Inference(anyhow::anyhow!(
                            "reranker returned a non-finite score"
                        )));
                    }
                    scores[batch_index * SCORE_BATCH_SIZE + result.index] = result.score;
                }
            }
            Ok(ScoreReport {
                scores,
                load_ms,
                inference_ms: inference_start.elapsed().as_millis() as u64,
            })
        })
    }
}

#[cfg(feature = "local-onnx")]
pub(super) use runtime::score_documents;

#[cfg(not(feature = "local-onnx"))]
pub(super) fn score_documents(
    verified: &VerifiedRerankerModel,
    _query: &str,
    _documents: &[String],
    _deadline: Instant,
) -> Result<ScoreReport, RerankModelError> {
    Err(RerankModelError::Load(anyhow::anyhow!(
        "local reranker runtime is not built; rebuild remem with the local-onnx feature to use {}",
        verified.manifest.model_id
    )))
}
