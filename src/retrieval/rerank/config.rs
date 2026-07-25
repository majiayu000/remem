use anyhow::{bail, Context, Result};
use toml_edit::{DocumentMut, Item};

pub(crate) const ENV_ENABLED: &str = "REMEM_RERANK_ENABLED";
pub(crate) const ENV_PRESET: &str = "REMEM_RERANK_PRESET";
pub(crate) const ENV_TOP_N: &str = "REMEM_RERANK_TOP_N";
pub(crate) const ENV_TOP_K: &str = "REMEM_RERANK_TOP_K";
pub(crate) const ENV_MAX_DOCUMENT_BYTES: &str = "REMEM_RERANK_MAX_DOCUMENT_BYTES";
pub(crate) const ENV_DEADLINE_MS: &str = "REMEM_RERANK_DEADLINE_MS";
pub(crate) const ENV_MODEL_DIR: &str = "REMEM_RERANK_MODEL_DIR";

/// Implementation-proposal defaults (maintainer-adjustable, see PR body):
/// rerank stays disabled until the separate default-on gate approves it.
const DEFAULT_TOP_N: usize = 50;
const DEFAULT_TOP_K: usize = 20;
const DEFAULT_MAX_DOCUMENT_BYTES: usize = 2048;
const DEFAULT_DEADLINE_MS: u64 = 1500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RerankConfig {
    pub enabled: bool,
    pub preset: String,
    pub top_n: usize,
    pub top_k: usize,
    pub max_document_bytes: usize,
    pub deadline_ms: u64,
    pub model_dir: Option<String>,
}

impl Default for RerankConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            preset: String::new(),
            top_n: DEFAULT_TOP_N,
            top_k: DEFAULT_TOP_K,
            max_document_bytes: DEFAULT_MAX_DOCUMENT_BYTES,
            deadline_ms: DEFAULT_DEADLINE_MS,
            model_dir: None,
        }
    }
}

pub(crate) fn resolve_rerank_config() -> Result<RerankConfig> {
    #[cfg(test)]
    let _test_env_guard = lock_test_env();
    resolve_rerank_config_unlocked()
}

/// Config resolution without acquiring the test env lock, for callers that
/// already hold it (test helpers).
pub(crate) fn resolve_rerank_config_unlocked() -> Result<RerankConfig> {
    let mut config = config_from_file()?.unwrap_or_default();
    apply_env_overrides(&mut config)?;
    validate_config(&config)?;
    Ok(config)
}

#[cfg(test)]
pub(crate) fn lock_test_env() -> crate::runtime_config::TestEnvGuard {
    crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire")
}

fn config_from_file() -> Result<Option<RerankConfig>> {
    let path = crate::runtime_config::config_path();
    if !path.exists() {
        return Ok(None);
    }
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let doc = content
        .parse::<DocumentMut>()
        .with_context(|| format!("parse {} as TOML", path.display()))?;
    let Some(table) = doc.get("rerank").and_then(Item::as_table) else {
        return Ok(None);
    };

    let mut config = RerankConfig::default();
    if let Some(enabled) = table.get("enabled") {
        config.enabled = enabled
            .as_bool()
            .context("rerank.enabled must be a boolean")?;
    }
    if let Some(preset) = optional_str(table, "preset") {
        config.preset = preset;
    }
    if let Some(top_n) = optional_usize(table, "top_n")? {
        config.top_n = top_n;
    }
    if let Some(top_k) = optional_usize(table, "top_k")? {
        config.top_k = top_k;
    }
    if let Some(max_document_bytes) = optional_usize(table, "max_document_bytes")? {
        config.max_document_bytes = max_document_bytes;
    }
    if let Some(deadline_ms) = optional_u64(table, "deadline_ms")? {
        config.deadline_ms = deadline_ms;
    }
    if let Some(model_dir) = optional_str(table, "model_dir") {
        config.model_dir = Some(model_dir);
    }
    Ok(Some(config))
}

fn apply_env_overrides(config: &mut RerankConfig) -> Result<()> {
    if let Some(enabled) = env_value(ENV_ENABLED) {
        config.enabled = parse_bool(&enabled, ENV_ENABLED)?;
    }
    if let Some(preset) = env_value(ENV_PRESET) {
        config.preset = preset;
    }
    if let Some(top_n) = env_value(ENV_TOP_N) {
        config.top_n = parse_positive_usize(&top_n, ENV_TOP_N)?;
    }
    if let Some(top_k) = env_value(ENV_TOP_K) {
        config.top_k = parse_positive_usize(&top_k, ENV_TOP_K)?;
    }
    if let Some(max_document_bytes) = env_value(ENV_MAX_DOCUMENT_BYTES) {
        config.max_document_bytes =
            parse_positive_usize(&max_document_bytes, ENV_MAX_DOCUMENT_BYTES)?;
    }
    if let Some(deadline_ms) = env_value(ENV_DEADLINE_MS) {
        config.deadline_ms = parse_positive_u64(&deadline_ms, ENV_DEADLINE_MS)?;
    }
    if let Some(model_dir) = env_value(ENV_MODEL_DIR) {
        config.model_dir = Some(model_dir);
    }
    Ok(())
}

pub(super) fn validate_config(config: &RerankConfig) -> Result<()> {
    if config.top_k == 0 {
        bail!("rerank.top_k must be positive");
    }
    if config.top_n < config.top_k {
        bail!(
            "rerank.top_n ({}) must be greater than or equal to rerank.top_k ({})",
            config.top_n,
            config.top_k
        );
    }
    if config.max_document_bytes == 0 {
        bail!("rerank.max_document_bytes must be positive");
    }
    if config.deadline_ms == 0 {
        bail!("rerank.deadline_ms must be positive");
    }
    // The preset string is validated against the closed preset set at the
    // inventory boundary; an unknown preset fails visibly there as well.
    super::inventory::RerankerPreset::parse(&config.preset)?;
    Ok(())
}

fn parse_bool(raw: &str, key: &str) -> Result<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        other => bail!("{key} must be a boolean, got {other}"),
    }
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
        .map(|item| {
            item.as_integer()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .with_context(|| format!("rerank.{key} must be a positive integer"))
        })
        .transpose()
}

fn optional_u64(table: &toml_edit::Table, key: &str) -> Result<Option<u64>> {
    table
        .get(key)
        .map(|item| {
            item.as_integer()
                .and_then(|value| u64::try_from(value).ok())
                .filter(|value| *value > 0)
                .with_context(|| format!("rerank.{key} must be a positive integer"))
        })
        .transpose()
}

fn parse_positive_usize(raw: &str, key: &str) -> Result<usize> {
    let value = raw
        .trim()
        .parse::<usize>()
        .with_context(|| format!("{key} must be a positive integer"))?;
    if value == 0 {
        bail!("{key} must be positive");
    }
    Ok(value)
}

fn parse_positive_u64(raw: &str, key: &str) -> Result<u64> {
    let value = raw
        .trim()
        .parse::<u64>()
        .with_context(|| format!("{key} must be a positive integer"))?;
    if value == 0 {
        bail!("{key} must be positive");
    }
    Ok(value)
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
