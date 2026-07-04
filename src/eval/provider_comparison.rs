use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use toml_edit::{value, DocumentMut, Item, Table};

use super::golden::{self, CategoryEvaluation, GoldenDataset};
use crate::retrieval::embedding::{
    self, EmbeddingConfig, EmbeddingProvider, EmbeddingProviderStatus,
};

pub const DEFAULT_DATASET_PATH: &str = "eval/golden.json";
pub const DEFAULT_REPORT_PATH: &str = "eval/provider-comparison/report.json";

const REPORT_VERSION: &str = "2026-07-04";
const PROVIDER_COMPARISON_SLICE: &str = "provider_comparison";
const QUERY_EMBEDDING_LATENCY_BUDGET_P95_MS: f64 = 1000.0;
const EXISTING_REGRESSION_BUDGET: f64 = 0.0;
const EPSILON: f64 = 0.000_001;

mod decision;
mod display;

use decision::build_default_decision;

const ENV_CONFIG: &str = "REMEM_CONFIG";
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
const DEFAULT_API_KEY_ENV: &str = "OPENAI_API_KEY";

const BASE_ENV_KEYS: &[&str] = &[
    ENV_CONFIG,
    ENV_PROVIDER,
    ENV_PROVIDER_LEGACY,
    ENV_MODEL,
    ENV_MODEL_LEGACY,
    ENV_BASE_URL,
    ENV_BASE_URL_LEGACY,
    ENV_DIMENSIONS,
    ENV_DIMENSIONS_LEGACY,
    ENV_API_KEY,
    ENV_API_KEY_LEGACY,
    ENV_API_KEY_ENV,
    ENV_TIMEOUT_SECS,
    ENV_FALLBACK,
    ENV_MODEL_DIR,
    DEFAULT_API_KEY_ENV,
];

#[derive(Debug, Clone)]
pub struct ProviderComparisonOptions {
    pub dataset_path: String,
    pub k: usize,
    pub json_out: String,
    pub allow_api: bool,
}

impl Default for ProviderComparisonOptions {
    fn default() -> Self {
        Self {
            dataset_path: DEFAULT_DATASET_PATH.to_string(),
            k: 5,
            json_out: DEFAULT_REPORT_PATH.to_string(),
            allow_api: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderComparisonReport {
    pub version: &'static str,
    pub generated_at_epoch: i64,
    pub dataset_path: String,
    pub k: usize,
    pub required_providers: Vec<&'static str>,
    pub provider_comparison_slice: &'static str,
    pub query_embedding_latency_budget_p95_ms: f64,
    pub existing_regression_budget: f64,
    pub providers: Vec<ProviderComparisonRow>,
    pub default_decision: DefaultDecision,
    pub notes: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderComparisonRow {
    pub provider: &'static str,
    pub configured_provider: String,
    pub active_provider: String,
    pub fallback_provider: Option<String>,
    pub model_id: Option<String>,
    pub dimensions: Option<usize>,
    pub available: bool,
    pub degraded: bool,
    pub disabled: bool,
    pub unavailable_reason: Option<String>,
    pub provider_config: ProviderConfigSummary,
    pub query_embedding_latency_p95_ms: Option<f64>,
    pub query_embedding_latency_samples: usize,
    pub overall: Option<CategoryEvaluation>,
    pub existing_slices: Option<CategoryEvaluation>,
    pub existing_slice_details: BTreeMap<String, CategoryEvaluation>,
    pub provider_comparison_slice: Option<CategoryEvaluation>,
    pub query_summaries: Vec<ProviderQuerySummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderConfigSummary {
    pub provider: String,
    pub fallback: Option<String>,
    pub model: String,
    pub base_url: String,
    pub dimensions: Option<usize>,
    pub api_key_env: String,
    pub model_dir: Option<String>,
    pub timeout_secs: u64,
    pub api_calls_allowed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderQuerySummary {
    pub id: String,
    pub slice: String,
    pub status: String,
    pub result_count: usize,
    pub retrieved_ids: Vec<i64>,
    pub matched_refs: usize,
    pub expected_refs: usize,
    pub retrieval_latency_ms: f64,
    pub query_embedding_latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefaultDecision {
    pub change_default: bool,
    pub decision: DefaultDecisionKind,
    pub decision_reason: String,
    pub criteria: DefaultFlipCriteria,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultDecisionKind {
    KeepFeatureHash,
    FlipToLocal,
}

#[derive(Debug, Clone, Serialize)]
pub struct DefaultFlipCriteria {
    pub local_available: bool,
    pub api_reference_available: bool,
    pub provider_comparison_slice_present: bool,
    pub provider_comparison_slice_improves: bool,
    pub existing_slices_within_budget: bool,
    pub query_embedding_latency_within_budget: bool,
}

pub fn run_provider_comparison_eval(
    options: ProviderComparisonOptions,
) -> Result<ProviderComparisonReport> {
    let dataset = golden::load_dataset(&options.dataset_path)?;
    run_provider_comparison_dataset(options, dataset)
}

fn run_provider_comparison_dataset(
    options: ProviderComparisonOptions,
    dataset: GoldenDataset,
) -> Result<ProviderComparisonReport> {
    let _env_guard = crate::runtime_config::ENV_LOCK
        .lock()
        .map_err(|_| anyhow!("embedding provider env lock poisoned"))?;
    run_provider_comparison_dataset_locked(options, dataset)
}

fn run_provider_comparison_dataset_locked(
    options: ProviderComparisonOptions,
    dataset: GoldenDataset,
) -> Result<ProviderComparisonReport> {
    if !dataset.has_fixture_corpus() {
        bail!("provider comparison requires a fixture-backed golden dataset");
    }
    ensure_provider_comparison_slice(&dataset)?;

    let k = options.k.max(1);
    let base_config = embedding::resolve_embedding_config()?;
    let providers = vec![
        evaluate_provider(
            &dataset,
            &options.dataset_path,
            k,
            &base_config,
            EmbeddingProvider::FeatureHash,
            options.allow_api,
        )?,
        evaluate_provider(
            &dataset,
            &options.dataset_path,
            k,
            &base_config,
            EmbeddingProvider::Local,
            options.allow_api,
        )?,
        evaluate_provider(
            &dataset,
            &options.dataset_path,
            k,
            &base_config,
            EmbeddingProvider::OpenAi,
            options.allow_api,
        )?,
    ];
    let default_decision = build_default_decision(&providers);
    Ok(ProviderComparisonReport {
        version: REPORT_VERSION,
        generated_at_epoch: chrono::Utc::now().timestamp(),
        dataset_path: options.dataset_path,
        k,
        required_providers: vec!["feature-hash", "local", "api"],
        provider_comparison_slice: PROVIDER_COMPARISON_SLICE,
        query_embedding_latency_budget_p95_ms: QUERY_EMBEDDING_LATENCY_BUDGET_P95_MS,
        existing_regression_budget: EXISTING_REGRESSION_BUDGET,
        providers,
        default_decision,
        notes: vec![
            "Provider rows are forced without fallback so unavailable providers cannot pass by silently using another embedding space.",
            "Remote API calls are opt-in with --allow-api; the default reference run records API as unavailable instead of spending network/API budget.",
            "This report is evidence for GH-716 and does not change the default provider by itself.",
        ],
    })
}

fn ensure_provider_comparison_slice(dataset: &GoldenDataset) -> Result<()> {
    let count = dataset
        .queries
        .iter()
        .filter(|query| query.slice_label() == PROVIDER_COMPARISON_SLICE)
        .count();
    if count == 0 {
        bail!("provider comparison requires at least one provider_comparison golden query");
    }
    Ok(())
}

fn evaluate_provider(
    dataset: &GoldenDataset,
    dataset_path: &str,
    k: usize,
    base_config: &EmbeddingConfig,
    provider: EmbeddingProvider,
    allow_api: bool,
) -> Result<ProviderComparisonRow> {
    let forced_config = forced_provider_config(base_config, provider);
    if provider == EmbeddingProvider::OpenAi && !allow_api {
        return Ok(unavailable_row(
            provider,
            &forced_config,
            "api provider comparison skipped because --allow-api was not set",
            allow_api,
        ));
    }

    let _scope = ScopedEmbeddingConfig::activate(&forced_config, provider, allow_api)?;
    let status = embedding::embedding_provider_status_without_probe()
        .with_context(|| format!("resolve {} provider status", provider.label()))?;
    if let Some(reason) = status.unavailable_reason.clone() {
        ensure_optional_provider(provider, reason.as_str())?;
        return Ok(row_from_status_unavailable(
            provider,
            &forced_config,
            status,
            reason,
            allow_api,
        ));
    }
    if status.disabled {
        ensure_optional_provider(provider, "embedding provider is disabled")?;
        return Ok(row_from_status_unavailable(
            provider,
            &forced_config,
            status,
            "embedding provider is disabled".to_string(),
            allow_api,
        ));
    }

    let active_profile = embedding::configured_backfill_target()
        .with_context(|| format!("probe {} embedding profile", provider.label()))?;

    match evaluate_available_provider(dataset, k) {
        Ok(evaluation) => Ok(row_from_evaluation(
            provider,
            &forced_config,
            status,
            active_profile.model,
            active_profile.dimensions,
            evaluation,
            allow_api,
        )),
        Err(error) => {
            let reason = format!("provider comparison failed for {dataset_path}: {error}");
            ensure_optional_provider(provider, &reason)?;
            Ok(row_from_status_unavailable(
                provider,
                &forced_config,
                status,
                reason,
                allow_api,
            ))
        }
    }
}

fn ensure_optional_provider(provider: EmbeddingProvider, reason: &str) -> Result<()> {
    if provider == EmbeddingProvider::FeatureHash {
        bail!("feature-hash provider comparison baseline must be runnable: {reason}");
    }
    Ok(())
}

fn forced_provider_config(
    base_config: &EmbeddingConfig,
    provider: EmbeddingProvider,
) -> EmbeddingConfig {
    let defaults = EmbeddingConfig::default();
    let mut config = base_config.clone();
    config.provider = provider;
    config.fallback = None;
    match provider {
        EmbeddingProvider::FeatureHash => {
            config.model = embedding::FEATURE_HASH_EMBEDDING_MODEL.to_string();
            config.dimensions = Some(embedding::FEATURE_HASH_EMBEDDING_DIMENSIONS);
        }
        EmbeddingProvider::Local => {
            if base_config.provider != EmbeddingProvider::Local {
                config.model = defaults.model.clone();
            }
            config.dimensions = None;
        }
        EmbeddingProvider::OpenAi => {
            if !matches!(
                base_config.provider,
                EmbeddingProvider::Auto | EmbeddingProvider::OpenAi
            ) {
                config.model = defaults.model.clone();
                config.base_url = defaults.base_url.clone();
                config.dimensions = defaults.dimensions;
            }
        }
        EmbeddingProvider::Auto | EmbeddingProvider::Off => {}
    }
    config
}

fn unavailable_row(
    provider: EmbeddingProvider,
    config: &EmbeddingConfig,
    reason: impl Into<String>,
    allow_api: bool,
) -> ProviderComparisonRow {
    ProviderComparisonRow {
        provider: provider.label(),
        configured_provider: provider.label().to_string(),
        active_provider: provider.label().to_string(),
        fallback_provider: None,
        model_id: configured_model_id(provider, config),
        dimensions: config.dimensions,
        available: false,
        degraded: false,
        disabled: false,
        unavailable_reason: Some(reason.into()),
        provider_config: ProviderConfigSummary::from_config(config, allow_api),
        query_embedding_latency_p95_ms: None,
        query_embedding_latency_samples: 0,
        overall: None,
        existing_slices: None,
        existing_slice_details: BTreeMap::new(),
        provider_comparison_slice: None,
        query_summaries: vec![],
    }
}

fn row_from_status_unavailable(
    provider: EmbeddingProvider,
    config: &EmbeddingConfig,
    status: EmbeddingProviderStatus,
    reason: String,
    allow_api: bool,
) -> ProviderComparisonRow {
    ProviderComparisonRow {
        provider: provider.label(),
        configured_provider: status.configured_provider,
        active_provider: status.active_provider,
        fallback_provider: status.fallback_provider,
        model_id: status
            .active_model_id
            .or_else(|| configured_model_id(provider, config)),
        dimensions: status.active_dimensions.or(config.dimensions),
        available: false,
        degraded: status.degraded,
        disabled: status.disabled,
        unavailable_reason: Some(reason),
        provider_config: ProviderConfigSummary::from_config(config, allow_api),
        query_embedding_latency_p95_ms: None,
        query_embedding_latency_samples: 0,
        overall: None,
        existing_slices: None,
        existing_slice_details: BTreeMap::new(),
        provider_comparison_slice: None,
        query_summaries: vec![],
    }
}

fn row_from_evaluation(
    provider: EmbeddingProvider,
    config: &EmbeddingConfig,
    status: EmbeddingProviderStatus,
    active_model_id: String,
    active_dimensions: usize,
    evaluation: ProviderRunEvaluation,
    allow_api: bool,
) -> ProviderComparisonRow {
    let query_embedding_latency_p95_ms = (!evaluation.query_embedding_latencies_ms.is_empty())
        .then(|| golden::run::percentile(evaluation.query_embedding_latencies_ms.clone(), 95.0));
    ProviderComparisonRow {
        provider: provider.label(),
        configured_provider: status.configured_provider,
        active_provider: status.active_provider,
        fallback_provider: status.fallback_provider,
        model_id: Some(active_model_id),
        dimensions: Some(active_dimensions),
        available: true,
        degraded: status.degraded,
        disabled: status.disabled,
        unavailable_reason: None,
        provider_config: ProviderConfigSummary::from_config(config, allow_api),
        query_embedding_latency_p95_ms,
        query_embedding_latency_samples: evaluation.query_embedding_latencies_ms.len(),
        overall: Some(evaluation.overall),
        existing_slices: Some(evaluation.existing_slices),
        existing_slice_details: evaluation.existing_slice_details,
        provider_comparison_slice: Some(evaluation.provider_comparison_slice),
        query_summaries: evaluation.query_summaries,
    }
}

fn configured_model_id(provider: EmbeddingProvider, config: &EmbeddingConfig) -> Option<String> {
    match provider {
        EmbeddingProvider::FeatureHash => Some(embedding::FEATURE_HASH_EMBEDDING_MODEL.to_string()),
        EmbeddingProvider::OpenAi => Some(config.model.clone()),
        EmbeddingProvider::Local => embedding::configured_local_embedding_model_id(config).ok(),
        EmbeddingProvider::Auto | EmbeddingProvider::Off => None,
    }
}

impl ProviderConfigSummary {
    fn from_config(config: &EmbeddingConfig, allow_api: bool) -> Self {
        Self {
            provider: config.provider.label().to_string(),
            fallback: config.fallback.map(|provider| provider.label().to_string()),
            model: config.model.clone(),
            base_url: config.base_url.clone(),
            dimensions: config.dimensions,
            api_key_env: config.api_key_env.clone(),
            model_dir: config.model_dir.clone(),
            timeout_secs: config.timeout_secs,
            api_calls_allowed: allow_api,
        }
    }
}

struct ProviderRunEvaluation {
    overall: CategoryEvaluation,
    existing_slices: CategoryEvaluation,
    existing_slice_details: BTreeMap<String, CategoryEvaluation>,
    provider_comparison_slice: CategoryEvaluation,
    query_embedding_latencies_ms: Vec<f64>,
    query_summaries: Vec<ProviderQuerySummary>,
}

fn evaluate_available_provider(dataset: &GoldenDataset, k: usize) -> Result<ProviderRunEvaluation> {
    let conn = Connection::open_in_memory().context("open in-memory provider comparison DB")?;
    crate::migrate::run_migrations(&conn).context("migrate provider comparison DB")?;
    golden::run::seed_fixture_corpus(&conn, &dataset.corpus)
        .context("seed provider comparison fixture corpus")?;

    let mut overall = golden::run::CategoryAccumulator::default();
    let mut existing_slices = golden::run::CategoryAccumulator::default();
    let mut existing_slice_details = BTreeMap::<String, golden::run::CategoryAccumulator>::new();
    let mut provider_comparison_slice = golden::run::CategoryAccumulator::default();
    let mut query_embedding_latencies_ms = Vec::new();
    let mut query_summaries = Vec::with_capacity(dataset.queries.len());

    for query in &dataset.queries {
        let started = Instant::now();
        let (results, explain) = crate::retrieval::search::search_with_branch_explain(
            &conn,
            Some(&query.query),
            query.project.as_deref(),
            query.memory_type.as_deref(),
            k.max(10) as i64,
            0,
            false,
            query.branch.as_deref(),
        )?;
        let retrieval_latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        let query_embedding_latency_ms = explain
            .as_ref()
            .and_then(|explain| phase_latency_ms(&explain.timings, "query_embedding"));
        if let Some(latency_ms) = query_embedding_latency_ms {
            query_embedding_latencies_ms.push(latency_ms);
        }
        let query_tokens = golden::run::estimate_query_tokens(&query.query);
        let evaluation =
            golden::run::evaluate_query(query, &results, k, query_tokens, retrieval_latency_ms);

        golden::run::record_bucket(&mut overall, query, &evaluation);
        if query.slice_label() == PROVIDER_COMPARISON_SLICE {
            golden::run::record_bucket(&mut provider_comparison_slice, query, &evaluation);
        } else {
            golden::run::record_bucket(&mut existing_slices, query, &evaluation);
            golden::run::record_bucket(
                existing_slice_details
                    .entry(query.slice_label().to_string())
                    .or_default(),
                query,
                &evaluation,
            );
        }

        query_summaries.push(ProviderQuerySummary {
            id: evaluation.id,
            slice: evaluation.slice,
            status: evaluation.status.label().to_string(),
            result_count: evaluation.result_count,
            retrieved_ids: evaluation.retrieved_ids,
            matched_refs: evaluation.matched_refs,
            expected_refs: evaluation.expected_refs,
            retrieval_latency_ms,
            query_embedding_latency_ms,
        });
    }

    Ok(ProviderRunEvaluation {
        overall: golden::run::bucket_evaluation(overall),
        existing_slices: golden::run::bucket_evaluation(existing_slices),
        existing_slice_details: existing_slice_details
            .into_iter()
            .map(|(slice, bucket)| (slice, golden::run::bucket_evaluation(bucket)))
            .collect(),
        provider_comparison_slice: golden::run::bucket_evaluation(provider_comparison_slice),
        query_embedding_latencies_ms,
        query_summaries,
    })
}

fn phase_latency_ms(timings: &[crate::perf::PhaseTiming], phase: &str) -> Option<f64> {
    timings
        .iter()
        .find(|timing| timing.phase == phase)
        .map(|timing| timing.elapsed_ms as f64)
}

struct ScopedEmbeddingConfig {
    saved: Vec<(String, Option<String>)>,
    config_path: PathBuf,
}

impl ScopedEmbeddingConfig {
    fn activate(
        config: &EmbeddingConfig,
        provider: EmbeddingProvider,
        allow_api: bool,
    ) -> Result<Self> {
        let mut keys = BASE_ENV_KEYS
            .iter()
            .map(|key| (*key).to_string())
            .collect::<Vec<_>>();
        if !keys.iter().any(|key| key == &config.api_key_env) {
            keys.push(config.api_key_env.clone());
        }
        let saved = keys
            .iter()
            .map(|key| (key.clone(), std::env::var(key).ok()))
            .collect::<Vec<_>>();
        let config_path = temp_config_path(provider);
        write_temp_embedding_config(&config_path, config, provider)?;

        for key in &keys {
            unsafe { std::env::remove_var(key) };
        }
        unsafe {
            std::env::set_var(ENV_CONFIG, &config_path);
        }
        if provider == EmbeddingProvider::OpenAi && allow_api {
            restore_api_key_for_scoped_config(config, &saved);
        }

        Ok(Self { saved, config_path })
    }
}

impl Drop for ScopedEmbeddingConfig {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
        let _ = fs::remove_file(&self.config_path);
    }
}

fn restore_api_key_for_scoped_config(config: &EmbeddingConfig, saved: &[(String, Option<String>)]) {
    let saved_values = saved
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|value| (key.as_str(), value.as_str())))
        .collect::<BTreeMap<_, _>>();
    if let Some(value) = saved_values.get(ENV_API_KEY) {
        unsafe { std::env::set_var(ENV_API_KEY, value) };
    } else if let Some(value) = saved_values.get(ENV_API_KEY_LEGACY) {
        unsafe { std::env::set_var(ENV_API_KEY_LEGACY, value) };
    } else if let Some(value) = saved_values.get(config.api_key_env.as_str()) {
        unsafe { std::env::set_var(&config.api_key_env, value) };
    } else if let Some(value) = saved_values.get(DEFAULT_API_KEY_ENV) {
        unsafe { std::env::set_var(DEFAULT_API_KEY_ENV, value) };
    }
}

fn temp_config_path(provider: EmbeddingProvider) -> PathBuf {
    std::env::temp_dir().join(format!(
        "remem-provider-comparison-{}-{}-{}.toml",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        provider.label()
    ))
}

fn write_temp_embedding_config(
    path: &Path,
    config: &EmbeddingConfig,
    provider: EmbeddingProvider,
) -> Result<()> {
    let mut doc = DocumentMut::new();
    let mut table = Table::new();
    table["provider"] = value(provider.label());
    table["model"] = value(config.model.clone());
    table["base_url"] = value(config.base_url.clone());
    if let Some(dimensions) = config.dimensions {
        table["dimensions"] = value(dimensions as i64);
    }
    table["api_key_env"] = value(config.api_key_env.clone());
    if let Some(model_dir) = config.model_dir.as_deref() {
        table["model_dir"] = value(model_dir);
    }
    table["timeout_secs"] = value(config.timeout_secs as i64);
    doc["embeddings"] = Item::Table(table);
    fs::write(path, doc.to_string())
        .with_context(|| format!("write provider comparison config {}", path.display()))
}

#[cfg(test)]
mod tests;
