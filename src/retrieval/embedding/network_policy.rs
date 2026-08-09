use super::*;

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
    match active_provider(&config)? {
        ActiveEmbeddingProvider::Local => Ok(Some(fallback::embed_local_with_auto_race_fallback(
            query,
            LocalEmbeddingInputKind::Query,
            &config,
            &mut cache,
        )?)),
        ActiveEmbeddingProvider::FeatureHash => Ok(Some(TextEmbedding::new(
            FEATURE_HASH_EMBEDDING_MODEL,
            embed_text_local(query),
        )?)),
        ActiveEmbeddingProvider::OpenAi { .. } | ActiveEmbeddingProvider::Off => Ok(None),
    }
}
