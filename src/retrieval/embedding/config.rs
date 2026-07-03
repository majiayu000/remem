use anyhow::{bail, Context, Result};
use toml_edit::{DocumentMut, Item};

use super::{
    EmbeddingConfig, EmbeddingProvider, ENV_API_KEY_ENV, ENV_BASE_URL, ENV_BASE_URL_LEGACY,
    ENV_DIMENSIONS, ENV_DIMENSIONS_LEGACY, ENV_FALLBACK, ENV_MODEL, ENV_MODEL_DIR,
    ENV_MODEL_LEGACY, ENV_PROVIDER, ENV_PROVIDER_LEGACY, ENV_TIMEOUT_SECS,
};

pub(crate) fn resolve_embedding_config() -> Result<EmbeddingConfig> {
    let mut config = config_from_file()?.unwrap_or_default();
    apply_env_overrides(&mut config)?;
    validate_config(&config)?;
    Ok(config)
}

fn config_from_file() -> Result<Option<EmbeddingConfig>> {
    let path = crate::runtime_config::config_path();
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let doc = content
        .parse::<DocumentMut>()
        .with_context(|| format!("parse {} as TOML", path.display()))?;
    let Some(table) = doc.get("embeddings").and_then(Item::as_table) else {
        return Ok(None);
    };

    let mut config = EmbeddingConfig::default();
    if let Some(provider) = optional_str(table, "provider") {
        config.provider = EmbeddingProvider::parse(&provider)?;
    }
    if let Some(fallback) = optional_str(table, "fallback") {
        config.fallback = Some(EmbeddingProvider::parse(&fallback)?);
    }
    if let Some(model) = optional_str(table, "model") {
        config.model = model;
    }
    if let Some(base_url) = optional_str(table, "base_url") {
        config.base_url = base_url;
    }
    if let Some(dimensions) = optional_usize(table, "dimensions")? {
        config.dimensions = Some(dimensions);
    }
    if let Some(api_key_env) = optional_str(table, "api_key_env") {
        config.api_key_env = api_key_env;
    }
    if let Some(model_dir) = optional_str(table, "model_dir") {
        config.model_dir = Some(model_dir);
    }
    if let Some(timeout_secs) = optional_u64(table, "timeout_secs")? {
        config.timeout_secs = timeout_secs;
    }
    Ok(Some(config))
}

fn apply_env_overrides(config: &mut EmbeddingConfig) -> Result<()> {
    if let Some(provider) = env_value(ENV_PROVIDER).or_else(|| env_value(ENV_PROVIDER_LEGACY)) {
        config.provider = EmbeddingProvider::parse(&provider)?;
    }
    if let Some(fallback) = env_value(ENV_FALLBACK) {
        config.fallback = Some(EmbeddingProvider::parse(&fallback)?);
    }
    if let Some(model) = env_value(ENV_MODEL).or_else(|| env_value(ENV_MODEL_LEGACY)) {
        config.model = model;
    }
    if let Some(base_url) = env_value(ENV_BASE_URL).or_else(|| env_value(ENV_BASE_URL_LEGACY)) {
        config.base_url = base_url;
    }
    if let Some(dimensions) = env_value(ENV_DIMENSIONS).or_else(|| env_value(ENV_DIMENSIONS_LEGACY))
    {
        config.dimensions = Some(parse_positive_usize(&dimensions, ENV_DIMENSIONS)?);
    }
    if let Some(api_key_env) = env_value(ENV_API_KEY_ENV) {
        config.api_key_env = api_key_env;
    }
    if let Some(model_dir) = env_value(ENV_MODEL_DIR) {
        config.model_dir = Some(model_dir);
    }
    if let Some(timeout_secs) = env_value(ENV_TIMEOUT_SECS) {
        config.timeout_secs = parse_positive_u64(&timeout_secs, ENV_TIMEOUT_SECS)?;
    }
    Ok(())
}

fn validate_config(config: &EmbeddingConfig) -> Result<()> {
    if config.model.trim().is_empty() {
        bail!("embeddings.model must not be empty");
    }
    if config.base_url.trim().is_empty() {
        bail!("embeddings.base_url must not be empty");
    }
    if config.api_key_env.trim().is_empty() {
        bail!("embeddings.api_key_env must not be empty");
    }
    if config.timeout_secs == 0 {
        bail!("embeddings.timeout_secs must be positive");
    }
    if config.fallback == Some(EmbeddingProvider::Auto) {
        bail!("embeddings.fallback must be a concrete provider");
    }
    Ok(())
}

fn optional_str(table: &toml_edit::Table, key: &str) -> Option<String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_usize(table: &toml_edit::Table, key: &str) -> Result<Option<usize>> {
    table
        .get(key)
        .map(|item| match item.as_integer() {
            Some(value) => usize::try_from(value)
                .ok()
                .filter(|value| *value > 0)
                .with_context(|| format!("embeddings.{key} must be positive")),
            None => item
                .as_str()
                .with_context(|| format!("embeddings.{key} must be an integer"))
                .and_then(|raw| parse_positive_usize(raw, key)),
        })
        .transpose()
}

fn optional_u64(table: &toml_edit::Table, key: &str) -> Result<Option<u64>> {
    table
        .get(key)
        .map(|item| match item.as_integer() {
            Some(value) => u64::try_from(value)
                .ok()
                .filter(|value| *value > 0)
                .with_context(|| format!("embeddings.{key} must be positive")),
            None => item
                .as_str()
                .with_context(|| format!("embeddings.{key} must be an integer"))
                .and_then(|raw| parse_positive_u64(raw, key)),
        })
        .transpose()
}

fn parse_positive_usize(raw: &str, key: &str) -> Result<usize> {
    raw.trim()
        .parse::<usize>()
        .with_context(|| format!("{key} must be a positive integer"))
        .and_then(|value| {
            if value == 0 {
                bail!("{key} must be positive");
            }
            Ok(value)
        })
}

fn parse_positive_u64(raw: &str, key: &str) -> Result<u64> {
    raw.trim()
        .parse::<u64>()
        .with_context(|| format!("{key} must be a positive integer"))
        .and_then(|value| {
            if value == 0 {
                bail!("{key} must be positive");
            }
            Ok(value)
        })
}

pub(super) fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
