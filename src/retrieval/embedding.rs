use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod config;
mod local_semantic;
mod status;

use config::env_value;
pub(crate) use config::resolve_embedding_config;
use local_semantic::LocalEmbeddingInputKind;
pub use local_semantic::{
    LocalEmbeddingDownloadReport, LocalEmbeddingInventoryReport, LocalEmbeddingModelInventory,
};
pub(crate) use status::is_embedding_provider_off_error;

pub const FEATURE_HASH_EMBEDDING_DIMENSIONS: usize = 768;
pub const FEATURE_HASH_EMBEDDING_MODEL: &str = "remem-local-feature-hash-v1";
pub const LOCAL_EMBEDDING_DIMENSIONS: usize = FEATURE_HASH_EMBEDDING_DIMENSIONS;
pub const LOCAL_EMBEDDING_MODEL: &str = FEATURE_HASH_EMBEDDING_MODEL;

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

#[derive(Debug, Default)]
pub(crate) struct EmbeddingFallbackCache {
    call_failure_fallback: Option<EmbeddingProvider>,
    call_failure_fallback_target: Option<EmbeddingBackfillTarget>,
}

impl EmbeddingFallbackCache {
    pub(crate) fn call_failure_fallback_target(&self) -> Option<EmbeddingBackfillTarget> {
        match self.call_failure_fallback {
            Some(EmbeddingProvider::Local) => self.call_failure_fallback_target.clone(),
            Some(EmbeddingProvider::FeatureHash) => Some(EmbeddingBackfillTarget {
                model: FEATURE_HASH_EMBEDDING_MODEL.to_string(),
                dimensions: FEATURE_HASH_EMBEDDING_DIMENSIONS,
            }),
            Some(EmbeddingProvider::Auto)
            | Some(EmbeddingProvider::OpenAi)
            | Some(EmbeddingProvider::Off) => None,
            None => None,
        }
    }
}

pub fn embed_query(query: &str) -> Result<TextEmbedding> {
    embed_text(query, LocalEmbeddingInputKind::Query)
}

pub(crate) fn embed_query_if_enabled(query: &str) -> Result<Option<TextEmbedding>> {
    match embed_query(query) {
        Ok(embedding) => Ok(Some(embedding)),
        Err(error) if is_embedding_provider_off_error(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn embed_memory(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Result<TextEmbedding> {
    let text = memory_embedding_text(title, content, memory_type, topic_key);
    embed_text(&text, LocalEmbeddingInputKind::Passage)
}

pub(crate) fn embed_memory_with_fallback_cache(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    cache: &mut EmbeddingFallbackCache,
) -> Result<TextEmbedding> {
    let text = memory_embedding_text(title, content, memory_type, topic_key);
    embed_text_with_fallback_cache(&text, LocalEmbeddingInputKind::Passage, cache)
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
    let mut cache = EmbeddingFallbackCache::default();
    configured_backfill_target_with_fallback_cache(&mut cache)
}

pub(crate) fn configured_backfill_target_with_fallback_cache(
    cache: &mut EmbeddingFallbackCache,
) -> Result<EmbeddingBackfillTarget> {
    if embedding_provider_status_without_probe()?.disabled {
        return Err(status::embedding_provider_off_error());
    }
    let probe = embed_text_with_fallback_cache(
        "remem embedding profile probe",
        LocalEmbeddingInputKind::Generic,
        cache,
    )?;
    Ok(EmbeddingBackfillTarget {
        model: probe.model().to_string(),
        dimensions: probe.dimensions(),
    })
}

pub fn embedding_provider_status() -> Result<EmbeddingProviderStatus> {
    let config = resolve_embedding_config()?;
    let mut status = status::resolve_provider_status(&config);
    status::probe_active_api_profile(&config, &mut status);
    Ok(status)
}

pub(crate) fn embedding_provider_status_without_probe() -> Result<EmbeddingProviderStatus> {
    let config = resolve_embedding_config()?;
    Ok(status::resolve_provider_status(&config))
}

pub(crate) fn configured_local_embedding_model_id(config: &EmbeddingConfig) -> Result<String> {
    local_semantic::configured_model_id(config)
}

pub fn download_local_embedding_model(model: Option<&str>) -> Result<LocalEmbeddingDownloadReport> {
    local_semantic::download_model(model)
}

pub fn local_embedding_inventory() -> Result<LocalEmbeddingInventoryReport> {
    local_semantic::inventory()
}

pub(crate) fn is_local_embedding_model_unavailable_error(error: &anyhow::Error) -> bool {
    local_semantic::is_model_unavailable_error(error)
}

fn embed_text(text: &str, kind: LocalEmbeddingInputKind) -> Result<TextEmbedding> {
    let mut cache = EmbeddingFallbackCache::default();
    embed_text_with_fallback_cache(text, kind, &mut cache)
}

fn embed_text_with_fallback_cache(
    text: &str,
    kind: LocalEmbeddingInputKind,
    cache: &mut EmbeddingFallbackCache,
) -> Result<TextEmbedding> {
    let config = resolve_embedding_config()?;
    if let Some(fallback) = cache.call_failure_fallback {
        return embed_with_cached_call_failure_fallback(text, kind, &config, fallback);
    }
    match active_provider(&config)? {
        ActiveEmbeddingProvider::Local => local_semantic::embed_text(text, &config, kind),
        ActiveEmbeddingProvider::FeatureHash => {
            TextEmbedding::new(FEATURE_HASH_EMBEDDING_MODEL, embed_text_local(text))
        }
        ActiveEmbeddingProvider::OpenAi { api_key } => embed_openai(text, &config, &api_key)
            .or_else(|error| embed_with_call_failure_fallback(text, kind, &config, error, cache)),
        ActiveEmbeddingProvider::Off => Err(status::embedding_provider_off_error()),
    }
}

fn embed_with_cached_call_failure_fallback(
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
        EmbeddingProvider::FeatureHash => {
            TextEmbedding::new(FEATURE_HASH_EMBEDDING_MODEL, embed_text_local(text))
        }
        EmbeddingProvider::Off => Err(status::embedding_provider_off_error()),
        EmbeddingProvider::OpenAi | EmbeddingProvider::Auto => {
            bail!("cached embedding fallback must be local, feature-hash, or off")
        }
    }
}

fn embed_with_call_failure_fallback(
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
            Ok(embedding)
        }
        EmbeddingProvider::FeatureHash => {
            cache.call_failure_fallback = Some(fallback_runtime.provider);
            cache.call_failure_fallback_target = Some(EmbeddingBackfillTarget {
                model: FEATURE_HASH_EMBEDDING_MODEL.to_string(),
                dimensions: FEATURE_HASH_EMBEDDING_DIMENSIONS,
            });
            TextEmbedding::new(FEATURE_HASH_EMBEDDING_MODEL, embed_text_local(text))
        }
        EmbeddingProvider::Off => Err(status::embedding_provider_off_error()),
        EmbeddingProvider::OpenAi | EmbeddingProvider::Auto => Err(error),
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
    let status = status::resolve_provider_status(config);
    if let Some(reason) = status.unavailable_reason {
        if status.active_provider == EmbeddingProvider::Local.label() {
            return Err(local_semantic::model_unavailable_error(reason));
        }
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

#[cfg(test)]
mod tests;
