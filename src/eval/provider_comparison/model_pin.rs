use anyhow::Result;

use crate::retrieval::embedding::{self, EmbeddingConfig, EmbeddingProvider};

pub(super) fn with_provider_model_pin<T>(
    provider: EmbeddingProvider,
    config: &EmbeddingConfig,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if provider == EmbeddingProvider::Local {
        return embedding::with_configured_model_read_lock(config, operation);
    }
    operation()
}
