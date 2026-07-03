use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item};

pub const LOCAL_EMBEDDING_DIMENSIONS: usize = 768;
pub const LOCAL_EMBEDDING_MODEL: &str = "remem-local-feature-hash-v1";

const DEFAULT_PROVIDER: EmbeddingProvider = EmbeddingProvider::Auto;
const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const OPENAI_DEFAULT_MODEL: &str = "text-embedding-3-small";
const DEFAULT_API_KEY_ENV: &str = "OPENAI_API_KEY";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

const ENV_PROVIDER: &str = "REMEM_EMBEDDINGS_PROVIDER";
const ENV_PROVIDER_LEGACY: &str = "REMEM_EMBEDDING_PROVIDER";
const ENV_MODEL: &str = "REMEM_EMBEDDINGS_MODEL";
const ENV_MODEL_LEGACY: &str = "REMEM_EMBEDDING_MODEL";
const ENV_BASE_URL: &str = "REMEM_EMBEDDINGS_BASE_URL";
const ENV_BASE_URL_LEGACY: &str = "REMEM_EMBEDDING_BASE_URL";
const ENV_DIMENSIONS: &str = "REMEM_EMBEDDINGS_DIMENSIONS";
const ENV_DIMENSIONS_LEGACY: &str = "REMEM_EMBEDDING_DIMENSIONS";
const ENV_API_KEY: &str = "REMEM_EMBEDDINGS_API_KEY";
const ENV_API_KEY_LEGACY: &str = "REMEM_EMBEDDING_API_KEY";
const ENV_API_KEY_ENV: &str = "REMEM_EMBEDDINGS_API_KEY_ENV";
const ENV_TIMEOUT_SECS: &str = "REMEM_EMBEDDINGS_TIMEOUT_SECS";
const ENV_FALLBACK: &str = "REMEM_EMBEDDINGS_FALLBACK";
const ENV_MODEL_DIR: &str = "REMEM_EMBEDDINGS_MODEL_DIR";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    Auto,
    Local,
    FeatureHash,
    OpenAi,
    Off,
}

impl EmbeddingProvider {
    fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "local" => Ok(Self::Local),
            "feature-hash" | "feature_hash" | "offline" => Ok(Self::FeatureHash),
            "api" | "openai" | "openai-compatible" | "openai_compatible" => Ok(Self::OpenAi),
            "off" | "disabled" | "none" => Ok(Self::Off),
            other => bail!("unknown embeddings.provider: {other}"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Local => "local",
            Self::FeatureHash => "feature-hash",
            Self::OpenAi => "api",
            Self::Off => "off",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,
    pub fallback: Option<EmbeddingProvider>,
    pub model: String,
    pub base_url: String,
    pub dimensions: Option<usize>,
    pub api_key_env: String,
    pub model_dir: Option<String>,
    pub timeout_secs: u64,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: DEFAULT_PROVIDER,
            fallback: None,
            model: OPENAI_DEFAULT_MODEL.to_string(),
            base_url: OPENAI_DEFAULT_BASE_URL.to_string(),
            dimensions: None,
            api_key_env: DEFAULT_API_KEY_ENV.to_string(),
            model_dir: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingProviderStatus {
    pub configured_provider: String,
    pub fallback_provider: Option<String>,
    pub active_provider: String,
    pub active_model_id: Option<String>,
    pub active_dimensions: Option<usize>,
    pub degraded: bool,
    pub disabled: bool,
    pub unavailable_reason: Option<String>,
    pub degradation_reason: Option<String>,
    pub model_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextEmbedding {
    model: String,
    values: Vec<f32>,
}

impl TextEmbedding {
    pub fn new(model: impl Into<String>, values: Vec<f32>) -> Result<Self> {
        let model = model.into();
        if model.trim().is_empty() {
            bail!("embedding model must not be empty");
        }
        validate_embedding_values(&values)?;
        Ok(Self { model, values })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn dimensions(&self) -> usize {
        self.values.len()
    }

    pub fn profile(&self) -> EmbeddingProfile<'_> {
        EmbeddingProfile {
            model: &self.model,
            dimensions: self.values.len(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingProfile<'a> {
    pub model: &'a str,
    pub dimensions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingBackfillTarget {
    pub model: String,
    pub dimensions: usize,
}

pub fn embed_query(query: &str) -> Result<TextEmbedding> {
    embed_text(query)
}

pub fn embed_memory(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Result<TextEmbedding> {
    let text = memory_embedding_text(title, content, memory_type, topic_key);
    embed_text(&text)
}

pub fn embed_query_text_local(query: &str) -> Vec<f32> {
    embed_text_local(query)
}

pub fn embed_memory_text_local(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Vec<f32> {
    embed_text_local(&memory_embedding_text(
        title,
        content,
        memory_type,
        topic_key,
    ))
}

pub fn embedding_content_hash(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(memory_type.as_bytes());
    hasher.update([0]);
    if let Some(topic_key) = topic_key {
        hasher.update(topic_key.as_bytes());
    }
    hasher.update([0]);
    hasher.update(title.as_bytes());
    hasher.update([0]);
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn configured_backfill_target() -> Result<EmbeddingBackfillTarget> {
    if embedding_provider_status()?.disabled {
        bail!("embedding provider is off");
    }
    let probe = embed_text("remem embedding profile probe")?;
    Ok(EmbeddingBackfillTarget {
        model: probe.model().to_string(),
        dimensions: probe.dimensions(),
    })
}

pub fn embedding_provider_status() -> Result<EmbeddingProviderStatus> {
    let config = resolve_embedding_config()?;
    Ok(resolve_provider_status(&config))
}

fn embed_text(text: &str) -> Result<TextEmbedding> {
    let config = resolve_embedding_config()?;
    match active_provider(&config)? {
        ActiveEmbeddingProvider::Local | ActiveEmbeddingProvider::FeatureHash => {
            TextEmbedding::new(LOCAL_EMBEDDING_MODEL, embed_text_local(text))
        }
        ActiveEmbeddingProvider::OpenAi { api_key } => embed_openai(text, &config, &api_key),
        ActiveEmbeddingProvider::Off => bail!("embedding provider is off"),
    }
}

fn memory_embedding_text(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> String {
    let mut text = String::new();
    text.push_str(memory_type);
    text.push('\n');
    if let Some(topic_key) = topic_key {
        text.push_str(topic_key);
        text.push('\n');
    }
    text.push_str(title);
    text.push('\n');
    text.push_str(content);
    text
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActiveEmbeddingProvider {
    Local,
    FeatureHash,
    OpenAi { api_key: String },
    Off,
}

fn active_provider(config: &EmbeddingConfig) -> Result<ActiveEmbeddingProvider> {
    let status = resolve_provider_status(config);
    if let Some(reason) = status.unavailable_reason {
        bail!("{reason}");
    }
    match EmbeddingProvider::parse(&status.active_provider)? {
        EmbeddingProvider::Local => Ok(ActiveEmbeddingProvider::Local),
        EmbeddingProvider::FeatureHash => Ok(ActiveEmbeddingProvider::FeatureHash),
        EmbeddingProvider::OpenAi => Ok(ActiveEmbeddingProvider::OpenAi {
            api_key: configured_api_key(config)?.with_context(|| {
                format!(
                    "embedding provider api requires {ENV_API_KEY} or {}",
                    config.api_key_env
                )
            })?,
        }),
        EmbeddingProvider::Off => Ok(ActiveEmbeddingProvider::Off),
        EmbeddingProvider::Auto => bail!("auto must resolve to a concrete embedding provider"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderRuntime {
    provider: EmbeddingProvider,
    model_id: Option<String>,
    dimensions: Option<usize>,
    disabled: bool,
    unavailable_reason: Option<String>,
}

fn resolve_provider_status(config: &EmbeddingConfig) -> EmbeddingProviderStatus {
    let configured = config.provider;
    let mut runtime = provider_runtime(config, configured);
    let mut degraded = false;
    let mut degradation_reason = None;

    if let Some(reason) = runtime.unavailable_reason.clone() {
        degraded = true;
        if let Some(fallback) = config.fallback {
            let fallback_runtime = provider_runtime(config, fallback);
            if fallback_runtime.unavailable_reason.is_none() {
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
        model_dir: config.model_dir.clone(),
    }
}

fn provider_runtime(config: &EmbeddingConfig, provider: EmbeddingProvider) -> ProviderRuntime {
    match provider {
        EmbeddingProvider::Auto => match auto_api_key(config) {
            Ok(Some(_)) => provider_runtime(config, EmbeddingProvider::OpenAi),
            Ok(None) => provider_runtime(config, EmbeddingProvider::Local),
            Err(error) => unavailable_runtime(provider, error.to_string()),
        },
        EmbeddingProvider::Local => ProviderRuntime {
            provider: EmbeddingProvider::Local,
            model_id: Some(LOCAL_EMBEDDING_MODEL.to_string()),
            dimensions: Some(LOCAL_EMBEDDING_DIMENSIONS),
            disabled: false,
            unavailable_reason: None,
        },
        EmbeddingProvider::FeatureHash => ProviderRuntime {
            provider: EmbeddingProvider::FeatureHash,
            model_id: Some(LOCAL_EMBEDDING_MODEL.to_string()),
            dimensions: Some(LOCAL_EMBEDDING_DIMENSIONS),
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
                format!("requires {ENV_API_KEY} or {}", config.api_key_env),
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

fn auto_api_key(config: &EmbeddingConfig) -> Result<Option<String>> {
    if let Some(value) = env_value(ENV_API_KEY).or_else(|| env_value(ENV_API_KEY_LEGACY)) {
        return Ok(Some(value));
    }
    if config.api_key_env != DEFAULT_API_KEY_ENV {
        configured_api_key(config)
    } else {
        Ok(None)
    }
}

fn configured_api_key(config: &EmbeddingConfig) -> Result<Option<String>> {
    if let Some(value) = env_value(ENV_API_KEY).or_else(|| env_value(ENV_API_KEY_LEGACY)) {
        return Ok(Some(value));
    }
    Ok(std::env::var(&config.api_key_env)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

fn resolve_embedding_config() -> Result<EmbeddingConfig> {
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

#[derive(Debug, Serialize)]
struct OpenAiEmbeddingRequest<'a> {
    input: &'a str,
    model: &'a str,
    encoding_format: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
}

fn embed_openai(text: &str, config: &EmbeddingConfig, api_key: &str) -> Result<TextEmbedding> {
    if text.trim().is_empty() {
        bail!("embedding input must not be empty");
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs))
        .build()
        .context("build embedding HTTP client")?;
    let request = OpenAiEmbeddingRequest {
        input: text,
        model: &config.model,
        encoding_format: "float",
        dimensions: config.dimensions,
    };
    let url = format!("{}/embeddings", config.base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .bearer_auth(api_key)
        .json(&request)
        .send()
        .with_context(|| format!("call embedding provider at {url}"))?;
    let status = response.status();
    let body = response
        .text()
        .context("read embedding provider response body")?;
    if !status.is_success() {
        bail!(
            "embedding provider returned HTTP {status}: {}",
            truncate_error_body(&body)
        );
    }
    parse_openai_embedding_response(&body, &config.model)
}

fn parse_openai_embedding_response(body: &str, fallback_model: &str) -> Result<TextEmbedding> {
    let response: OpenAiEmbeddingResponse =
        serde_json::from_str(body).context("parse embedding provider response")?;
    let mut data = response.data.into_iter();
    let first = data
        .next()
        .context("embedding provider response did not include data[0]")?;
    if data.next().is_some() {
        bail!("embedding provider returned multiple embeddings for single input");
    }
    TextEmbedding::new(
        response.model.unwrap_or_else(|| fallback_model.to_string()),
        first.embedding,
    )
}

fn truncate_error_body(body: &str) -> String {
    const MAX: usize = 500;
    if body.len() <= MAX {
        body.to_string()
    } else {
        let mut end = MAX;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &body[..end])
    }
}

fn validate_embedding_values(values: &[f32]) -> Result<()> {
    if values.is_empty() {
        bail!("embedding vector must not be empty");
    }
    if values.iter().any(|value| !value.is_finite()) {
        bail!("embedding vector contains non-finite values");
    }
    Ok(())
}

fn embed_text_local(text: &str) -> Vec<f32> {
    let normalized = text.to_lowercase();
    let mut vector = vec![0.0f32; LOCAL_EMBEDDING_DIMENSIONS];
    for token in semantic_tokens(&normalized) {
        add_feature(&mut vector, &format!("token:{token}"), 1.0);
    }
    for ngram in char_ngrams(&normalized) {
        add_feature(&mut vector, &format!("ngram:{ngram}"), 0.35);
    }
    for (concept, phrases) in semantic_concepts() {
        if phrases.iter().any(|phrase| normalized.contains(phrase)) {
            add_feature(&mut vector, &format!("concept:{concept}"), 4.0);
        }
    }
    normalize(&mut vector);
    vector
}

fn semantic_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || is_cjk(ch) {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn char_ngrams(text: &str) -> Vec<String> {
    let chars: Vec<char> = text
        .chars()
        .filter(|ch| ch.is_alphanumeric() || is_cjk(*ch))
        .collect();
    let mut grams = Vec::new();
    for width in [2usize, 3] {
        if chars.len() < width {
            continue;
        }
        grams.extend(
            chars
                .windows(width)
                .map(|window| window.iter().collect::<String>()),
        );
    }
    grams
}

fn add_feature(vector: &mut [f32], feature: &str, weight: f32) {
    let digest = Sha256::digest(feature.as_bytes());
    for offset in [0usize, 8, 16] {
        let raw = u64::from_le_bytes([
            digest[offset],
            digest[offset + 1],
            digest[offset + 2],
            digest[offset + 3],
            digest[offset + 4],
            digest[offset + 5],
            digest[offset + 6],
            digest[offset + 7],
        ]);
        let idx = raw as usize % vector.len();
        let sign = if raw & 1 == 0 { 1.0 } else { -1.0 };
        vector[idx] += weight * sign;
    }
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' |
        '\u{3400}'..='\u{4DBF}' |
        '\u{F900}'..='\u{FAFF}'
    )
}

fn semantic_concepts() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        (
            "data-security",
            &[
                "sqlcipher",
                "encrypt",
                "encrypted",
                "encryption",
                "secret",
                "secrets",
                "credential",
                "credentials",
                "private",
                "confidential",
                "protect",
                "protected",
                "at rest",
                "persisted data",
                "加密",
                "密钥",
            ],
        ),
        (
            "transcript-capture",
            &[
                "transcript",
                "raw archive",
                "raw message",
                "hook fallback",
                "assistant message",
                "conversation capture",
                "jsonl",
                "会话",
                "原始消息",
            ],
        ),
        (
            "retrieval-quality",
            &[
                "semantic",
                "embedding",
                "vector",
                "recall",
                "search quality",
                "paraphrase",
                "检索",
                "语义",
                "召回",
                "向量",
            ],
        ),
        (
            "current-state",
            &[
                "current decision",
                "current state",
                "supersede",
                "supersedes",
                "stale",
                "replacement",
                "现在",
                "当前",
                "替代",
            ],
        ),
        (
            "compression",
            &[
                "compress",
                "compression",
                "compaction",
                "summarize",
                "compressed",
                "压缩",
                "摘要",
                "总结",
            ],
        ),
    ]
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

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests;
