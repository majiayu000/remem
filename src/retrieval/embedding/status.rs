use super::{
    configured_api_key, embed_openai, EmbeddingConfig, EmbeddingProvider, EmbeddingProviderStatus,
    FEATURE_HASH_EMBEDDING_DIMENSIONS, FEATURE_HASH_EMBEDDING_MODEL,
};

#[derive(Debug)]
struct EmbeddingProviderOffError {
    cause: Option<String>,
}

impl std::fmt::Display for EmbeddingProviderOffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.cause.as_deref() {
            Some(cause) => write!(f, "embedding provider is off: {cause}"),
            None => f.write_str("embedding provider is off"),
        }
    }
}

impl std::error::Error for EmbeddingProviderOffError {}

pub(crate) fn is_embedding_provider_off_error(error: &anyhow::Error) -> bool {
    error.downcast_ref::<EmbeddingProviderOffError>().is_some()
}

pub(super) fn embedding_provider_off_error() -> anyhow::Error {
    EmbeddingProviderOffError { cause: None }.into()
}

pub(super) fn embedding_provider_off_error_with_cause(cause: String) -> anyhow::Error {
    EmbeddingProviderOffError { cause: Some(cause) }.into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderRuntime {
    pub(super) provider: EmbeddingProvider,
    pub(super) model_id: Option<String>,
    pub(super) dimensions: Option<usize>,
    pub(super) disabled: bool,
    pub(super) unavailable_reason: Option<String>,
}

pub(super) fn resolve_provider_status(config: &EmbeddingConfig) -> EmbeddingProviderStatus {
    let configured = config.provider;
    let mut runtime = provider_runtime(config, configured);
    let mut degraded = false;
    let mut degradation_reason = None;

    if let Some(reason) = runtime.unavailable_reason.clone() {
        degraded = true;
        if let Some(fallback) = config.fallback {
            let fallback_runtime = provider_runtime(config, fallback);
            if fallback == EmbeddingProvider::Off {
                let message = format!(
                    "configured embedding provider {} unavailable: {}; using fallback off; fallback off disabled provider fallback",
                    configured.label(),
                    reason
                );
                degradation_reason = Some(message.clone());
                runtime = disabled_runtime();
            } else if fallback_runtime.unavailable_reason.is_none() {
                let message = format!(
                    "configured embedding provider {} unavailable: {}; using fallback {}",
                    configured.label(),
                    reason,
                    fallback.label()
                );
                crate::log::error("embedding", &message);
                degradation_reason = Some(message);
                runtime = fallback_runtime;
            } else {
                degradation_reason = Some(format!(
                    "configured embedding provider {} unavailable: {}; fallback {} unavailable: {}",
                    configured.label(),
                    reason,
                    fallback.label(),
                    fallback_runtime
                        .unavailable_reason
                        .as_deref()
                        .unwrap_or("unknown")
                ));
            }
        } else {
            degradation_reason = Some(format!(
                "configured embedding provider {} unavailable: {}",
                configured.label(),
                reason
            ));
        }
    }

    EmbeddingProviderStatus {
        configured_provider: configured.label().to_string(),
        fallback_provider: config.fallback.map(|provider| provider.label().to_string()),
        active_provider: runtime.provider.label().to_string(),
        active_model_id: runtime.model_id,
        active_dimensions: runtime.dimensions,
        degraded,
        disabled: runtime.disabled,
        unavailable_reason: runtime.unavailable_reason,
        degradation_reason,
        model_dir: Some(
            super::local_semantic::model_root(config)
                .display()
                .to_string(),
        ),
    }
}

pub(super) fn probe_active_api_profile(
    config: &EmbeddingConfig,
    status: &mut EmbeddingProviderStatus,
) {
    if status.unavailable_reason.is_some()
        || status.active_provider != EmbeddingProvider::OpenAi.label()
    {
        return;
    }
    let api_key = match configured_api_key(config) {
        Ok(Some(api_key)) => api_key,
        Ok(None) => {
            status.unavailable_reason = Some(format!(
                "requires {} or {}",
                super::ENV_API_KEY,
                config.api_key_env
            ));
            status.active_model_id = None;
            status.active_dimensions = None;
            return;
        }
        Err(error) => {
            status.unavailable_reason = Some(error.to_string());
            status.active_model_id = None;
            status.active_dimensions = None;
            return;
        }
    };
    match embed_openai("remem embedding profile probe", config, &api_key) {
        Ok(profile) => {
            status.active_model_id = Some(profile.model().to_string());
            status.active_dimensions = Some(profile.dimensions());
        }
        Err(error) => apply_api_probe_failure_status(config, status, error),
    }
}

fn apply_api_probe_failure_status(
    config: &EmbeddingConfig,
    status: &mut EmbeddingProviderStatus,
    error: anyhow::Error,
) {
    if let Some(fallback) = config.fallback {
        let fallback_runtime = provider_runtime(config, fallback);
        status.degraded = true;
        if fallback == EmbeddingProvider::Off {
            let message = format!(
                "configured embedding provider api unavailable: {}; using fallback off; fallback off disabled provider fallback",
                error
            );
            status.degradation_reason = Some(message.clone());
            status.active_provider = EmbeddingProvider::Off.label().to_string();
            status.active_model_id = None;
            status.active_dimensions = None;
            status.disabled = true;
            status.unavailable_reason = None;
        } else if fallback_runtime.unavailable_reason.is_none() {
            let message = format!(
                "configured embedding provider api unavailable: {}; using fallback {}",
                error,
                fallback.label()
            );
            crate::log::error("embedding", &message);
            status.degradation_reason = Some(message);
            status.active_provider = fallback_runtime.provider.label().to_string();
            status.active_model_id = fallback_runtime.model_id;
            status.active_dimensions = fallback_runtime.dimensions;
            status.disabled = fallback_runtime.disabled;
            status.unavailable_reason = fallback_runtime.unavailable_reason;
        } else {
            status.degradation_reason = Some(format!(
                "configured embedding provider api unavailable: {}; fallback {} unavailable: {}",
                error,
                fallback.label(),
                fallback_runtime
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("unknown")
            ));
            status.active_model_id = None;
            status.active_dimensions = None;
            status.disabled = false;
            status.unavailable_reason = fallback_runtime.unavailable_reason;
        }
        return;
    }

    status.unavailable_reason = Some(format!("embedding provider api unavailable: {error}"));
    status.active_model_id = None;
    status.active_dimensions = None;
}

pub(super) fn provider_runtime(
    config: &EmbeddingConfig,
    provider: EmbeddingProvider,
) -> ProviderRuntime {
    match provider {
        EmbeddingProvider::Auto => match super::auto_api_key(config) {
            Ok(Some(_)) => provider_runtime(config, EmbeddingProvider::OpenAi),
            Ok(None) => provider_runtime(config, EmbeddingProvider::FeatureHash),
            Err(error) => unavailable_runtime(provider, error.to_string()),
        },
        EmbeddingProvider::Local => match super::local_semantic::installed_model_profile(config) {
            Ok(profile) => ProviderRuntime {
                provider: EmbeddingProvider::Local,
                model_id: Some(profile.model),
                dimensions: Some(profile.dimensions),
                disabled: false,
                unavailable_reason: None,
            },
            Err(error) => unavailable_runtime(provider, error.to_string()),
        },
        EmbeddingProvider::FeatureHash => ProviderRuntime {
            provider: EmbeddingProvider::FeatureHash,
            model_id: Some(FEATURE_HASH_EMBEDDING_MODEL.to_string()),
            dimensions: Some(FEATURE_HASH_EMBEDDING_DIMENSIONS),
            disabled: false,
            unavailable_reason: None,
        },
        EmbeddingProvider::OpenAi => match configured_api_key(config) {
            Ok(Some(_)) => ProviderRuntime {
                provider: EmbeddingProvider::OpenAi,
                model_id: Some(config.model.clone()),
                dimensions: config.dimensions,
                disabled: false,
                unavailable_reason: None,
            },
            Ok(None) => unavailable_runtime(
                provider,
                format!("requires {} or {}", super::ENV_API_KEY, config.api_key_env),
            ),
            Err(error) => unavailable_runtime(provider, error.to_string()),
        },
        EmbeddingProvider::Off => ProviderRuntime {
            provider: EmbeddingProvider::Off,
            model_id: None,
            dimensions: None,
            disabled: true,
            unavailable_reason: None,
        },
    }
}

fn unavailable_runtime(provider: EmbeddingProvider, reason: String) -> ProviderRuntime {
    ProviderRuntime {
        provider,
        model_id: None,
        dimensions: None,
        disabled: false,
        unavailable_reason: Some(reason),
    }
}

fn disabled_runtime() -> ProviderRuntime {
    ProviderRuntime {
        provider: EmbeddingProvider::Off,
        model_id: None,
        dimensions: None,
        disabled: true,
        unavailable_reason: None,
    }
}
