use super::*;

const LOCAL_ONLY_EMBEDDING_POLICY_VERSION: &str = "context_bundle_local_embedding_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LocalOnlyEmbeddingMode {
    Local,
    FeatureHash,
    Skipped,
    Blocked,
}

#[derive(Serialize)]
struct LocalOnlyEmbeddingProfile<'a> {
    policy_version: &'static str,
    mode: LocalOnlyEmbeddingMode,
    active_provider: &'a str,
    active_model_id: Option<&'a str>,
    active_dimensions: Option<usize>,
}

fn local_only_embedding_mode(
    config: &EmbeddingConfig,
    status: &EmbeddingProviderStatus,
) -> LocalOnlyEmbeddingMode {
    if status.active_provider == EmbeddingProvider::OpenAi.label()
        || (status.active_provider == EmbeddingProvider::Off.label()
            && matches!(
                config.provider,
                EmbeddingProvider::OpenAi | EmbeddingProvider::Off
            ))
    {
        return LocalOnlyEmbeddingMode::Skipped;
    }
    match status.active_provider.as_str() {
        "local" if status.unavailable_reason.is_none() => LocalOnlyEmbeddingMode::Local,
        "feature-hash" if status.unavailable_reason.is_none() => {
            LocalOnlyEmbeddingMode::FeatureHash
        }
        _ => LocalOnlyEmbeddingMode::Blocked,
    }
}

/// SHA-256 over the effective local-only vector policy used by Context Bundle
/// v1. Only execution-relevant, non-secret fields participate: mode, resolved
/// provider, model identity (including the verified local artifact digest), and
/// dimensions. Invalid configuration is fingerprinted as a blocked profile so
/// the caller can still return the canonical blocked-bundle audit contract.
pub(crate) fn local_only_embedding_profile_fingerprint() -> String {
    #[cfg(test)]
    let _test_env_guard = config::lock_test_env();
    let canonical = match resolve_embedding_config() {
        Ok(config) => {
            let status = status::resolve_provider_status(&config);
            serde_json::to_vec(&LocalOnlyEmbeddingProfile {
                policy_version: LOCAL_ONLY_EMBEDDING_POLICY_VERSION,
                mode: local_only_embedding_mode(&config, &status),
                active_provider: &status.active_provider,
                active_model_id: status.active_model_id.as_deref(),
                active_dimensions: status.active_dimensions,
            })
            .expect("local-only embedding profile serialization is infallible")
        }
        Err(error) => serde_json::to_vec(&(
            LOCAL_ONLY_EMBEDDING_POLICY_VERSION,
            LocalOnlyEmbeddingMode::Blocked,
            error.to_string(),
        ))
        .expect("local-only embedding error profile serialization is infallible"),
    };
    let mut hasher = Sha256::new();
    hasher.update(canonical);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn embed_query_if_enabled(query: &str) -> Result<Option<TextEmbedding>> {
    match embed_query(query) {
        Ok(embedding) => Ok(Some(embedding)),
        Err(error) if is_embedding_provider_off_error(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Embed a query only when the resolved active provider is local.
///
/// Foreground contracts such as the MCP Context Bundle advertise that they do
/// not make network calls. Resolve the complete provider/fallback chain once
/// and execute only a selected local implementation directly, so an API
/// provider may safely degrade to its configured local fallback without any
/// chance of contacting the remote endpoint.
pub(crate) fn embed_query_local_only_if_enabled(query: &str) -> Result<Option<TextEmbedding>> {
    #[cfg(test)]
    let _test_env_guard = config::lock_test_env();
    let config = resolve_embedding_config()?;
    let mut cache = EmbeddingFallbackCache::default();
    let status = status::resolve_provider_status(&config);
    match local_only_embedding_mode(&config, &status) {
        LocalOnlyEmbeddingMode::Local => Ok(Some(fallback::embed_local_with_auto_race_fallback(
            query,
            LocalEmbeddingInputKind::Query,
            &config,
            &mut cache,
        )?)),
        LocalOnlyEmbeddingMode::FeatureHash => Ok(Some(TextEmbedding::new(
            FEATURE_HASH_EMBEDDING_MODEL,
            embed_text_local(query),
        )?)),
        LocalOnlyEmbeddingMode::Skipped => {
            // A local-only caller can still use lexical retrieval when the
            // resolved provider is remote, disabled, or an unavailable remote
            // provider has no usable local fallback. Never contact that API.
            Ok(None)
        }
        LocalOnlyEmbeddingMode::Blocked => {
            // Preserve typed local-provider and explicit fallback failures.
            // `active_provider` returns the diagnostic selected by the same
            // resolution used to fingerprint this blocked execution profile.
            let _ = active_provider(&config)?;
            unreachable!("blocked local-only embedding profile must fail provider activation")
        }
    }
}
