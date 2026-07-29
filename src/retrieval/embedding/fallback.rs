use anyhow::{bail, Result};

use super::{
    embed_text_local, local_semantic, status, EmbeddingBackfillTarget, EmbeddingConfig,
    EmbeddingFallbackCache, EmbeddingProvider, EmbeddingProviderStatus, LocalEmbeddingInputKind,
    TextEmbedding, FEATURE_HASH_EMBEDDING_DIMENSIONS, FEATURE_HASH_EMBEDDING_MODEL,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EmbeddingExecutionMetadata {
    pub configured_provider: String,
    /// Provider that produced this query's embedding, after any runtime fallback.
    pub active_provider: String,
    pub model: String,
    pub dimensions: usize,
    pub degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degradation_reason: Option<String>,
}

pub(super) fn embedding_execution_metadata(
    status_before: &EmbeddingProviderStatus,
    status_after: &EmbeddingProviderStatus,
    cache: &EmbeddingFallbackCache,
    embedding: &TextEmbedding,
) -> Result<EmbeddingExecutionMetadata> {
    let actual_provider = cache.execution_provider.ok_or_else(|| {
        anyhow::anyhow!("embedding execution completed without provider metadata")
    })?;
    let actual_provider = actual_provider.label();
    let mut reasons = Vec::new();
    append_unique_reason(&mut reasons, status_before.degradation_reason.as_deref());
    append_unique_reason(&mut reasons, cache.degradation_reason.as_deref());
    append_unique_reason(&mut reasons, status_after.degradation_reason.as_deref());
    let provider_changed = status_before.active_provider != status_after.active_provider
        || status_before.active_provider != actual_provider;
    if provider_changed {
        reasons.push(format!(
            "embedding provider changed during query execution: initial={}, final={}, actual={actual_provider}",
            status_before.active_provider, status_after.active_provider
        ));
    }
    let degraded =
        status_before.degraded || status_after.degraded || provider_changed || !reasons.is_empty();
    if degraded && reasons.is_empty() {
        reasons.push("embedding execution was degraded".to_string());
    }
    let degradation_reason = (!reasons.is_empty())
        .then(|| crate::adapter::common::redact_hook_payload_preview(&reasons.join("; "), 1024));
    Ok(EmbeddingExecutionMetadata {
        configured_provider: status_before.configured_provider.clone(),
        active_provider: actual_provider.to_string(),
        model: embedding.model().to_string(),
        dimensions: embedding.dimensions(),
        degraded,
        degradation_reason,
    })
}

fn append_unique_reason(reasons: &mut Vec<String>, reason: Option<&str>) {
    if let Some(reason) = reason {
        if !reasons.iter().any(|existing| existing == reason) {
            reasons.push(reason.to_string());
        }
    }
}

pub(super) fn embed_local_with_auto_race_fallback(
    text: &str,
    kind: LocalEmbeddingInputKind,
    config: &EmbeddingConfig,
    cache: &mut EmbeddingFallbackCache,
) -> Result<TextEmbedding> {
    match local_semantic::embed_text(text, config, kind) {
        Ok(embedding) => {
            cache.execution_provider = Some(EmbeddingProvider::Local);
            Ok(embedding)
        }
        Err(error)
            if config.provider == EmbeddingProvider::Auto
                && local_semantic::is_model_unavailable_error(&error) =>
        {
            let message = format!(
                "automatic local embedding provider became unavailable: {error}; using feature-hash"
            );
            crate::log::error("embedding", &message);
            remember_feature_hash_fallback(cache, message);
            feature_hash_embedding(text)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn embed_with_cached_call_failure_fallback(
    text: &str,
    kind: LocalEmbeddingInputKind,
    config: &EmbeddingConfig,
    fallback: EmbeddingProvider,
) -> Result<TextEmbedding> {
    let fallback_runtime = status::provider_runtime(config, fallback);
    if let Some(reason) = fallback_runtime.unavailable_reason {
        bail!(
            "cached embedding fallback {} unavailable: {reason}",
            fallback.label()
        );
    }
    match fallback_runtime.provider {
        EmbeddingProvider::Local => local_semantic::embed_text(text, config, kind),
        EmbeddingProvider::FeatureHash => feature_hash_embedding(text),
        EmbeddingProvider::Off => Err(status::embedding_provider_off_error()),
        EmbeddingProvider::OpenAi | EmbeddingProvider::Auto => {
            bail!("cached embedding fallback must be local, feature-hash, or off")
        }
    }
}

pub(super) fn embed_with_call_failure_fallback(
    text: &str,
    kind: LocalEmbeddingInputKind,
    config: &EmbeddingConfig,
    error: anyhow::Error,
    cache: &mut EmbeddingFallbackCache,
) -> Result<TextEmbedding> {
    let Some(fallback) = config.fallback else {
        return Err(error);
    };
    let fallback_runtime = status::provider_runtime(config, fallback);
    if let Some(reason) = fallback_runtime.unavailable_reason {
        bail!(
            "embedding provider api failed: {error}; fallback {} unavailable: {reason}",
            fallback.label()
        );
    }
    let message = format!(
        "configured embedding provider api failed: {}; using fallback {}",
        error,
        fallback.label()
    );
    crate::log::error("embedding", &message);
    match fallback_runtime.provider {
        EmbeddingProvider::Local => {
            let embedding = local_semantic::embed_text(text, config, kind)?;
            cache.call_failure_fallback = Some(fallback_runtime.provider);
            cache.call_failure_fallback_target = Some(EmbeddingBackfillTarget {
                model: embedding.model().to_string(),
                dimensions: embedding.dimensions(),
            });
            cache.execution_provider = Some(EmbeddingProvider::Local);
            cache.degradation_reason = Some(message);
            Ok(embedding)
        }
        EmbeddingProvider::FeatureHash => {
            remember_feature_hash_fallback(cache, message);
            feature_hash_embedding(text)
        }
        EmbeddingProvider::Off => Err(status::embedding_provider_off_error_with_cause(format!(
            "embedding provider api failed: {error}; fallback off disabled provider fallback"
        ))),
        EmbeddingProvider::OpenAi | EmbeddingProvider::Auto => Err(error),
    }
}

fn remember_feature_hash_fallback(cache: &mut EmbeddingFallbackCache, reason: String) {
    cache.call_failure_fallback = Some(EmbeddingProvider::FeatureHash);
    cache.call_failure_fallback_target = Some(EmbeddingBackfillTarget {
        model: FEATURE_HASH_EMBEDDING_MODEL.to_string(),
        dimensions: FEATURE_HASH_EMBEDDING_DIMENSIONS,
    });
    cache.execution_provider = Some(EmbeddingProvider::FeatureHash);
    cache.degradation_reason = Some(reason);
}

fn feature_hash_embedding(text: &str) -> Result<TextEmbedding> {
    TextEmbedding::new(FEATURE_HASH_EMBEDDING_MODEL, embed_text_local(text))
}
