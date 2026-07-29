use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{EmbeddingConfig, TextEmbedding};

#[cfg(feature = "local-onnx")]
mod download;
#[cfg(feature = "local-onnx")]
mod fs_cleanup;
#[cfg(test)]
mod hash_counter;
mod manifest;
#[cfg(feature = "local-onnx")]
mod runtime;
#[cfg(test)]
mod test_support;
#[cfg(all(windows, feature = "local-onnx"))]
mod windows_cleanup;
#[cfg(windows)]
mod windows_model_root;
#[cfg(windows)]
mod windows_security;

#[cfg(test)]
use hash_counter::ModelFileHashCounter;
use manifest::read_verified_manifest_compatible;
#[cfg(feature = "local-onnx")]
use manifest::with_model_read_lock;
#[cfg(test)]
use manifest::{collect_model_artifacts, write_manifest};
#[cfg(feature = "local-onnx")]
use manifest::{open_or_create_model_lock, read_verified_manifest_unlocked};
#[cfg(test)]
pub(crate) use test_support::install_test_model;
#[cfg(all(test, feature = "local-onnx"))]
pub(crate) use test_support::{
    fail_next_test_model_embed_generic, fail_next_test_model_embed_unavailable,
    fail_test_model_runtime_readiness, install_test_model_v1, install_untrusted_test_model,
    test_model_runtime_file,
};

pub(super) const DEFAULT_LOCAL_SEMANTIC_DIMENSIONS: usize = 384;
pub(super) const DEFAULT_LOCAL_SEMANTIC_MODEL: &str = "fastembed-intfloat-multilingual-e5-small-v1";

const MANIFEST_FILE: &str = "remem-model-manifest.json";
const MANIFEST_SCHEMA_VERSION: u32 = 2;
const FASTEMBED_RUNTIME: &str = "fastembed-rs/onnxruntime";
const HUGGING_FACE_BASE_URL: &str = "https://huggingface.co";
#[cfg(feature = "local-onnx")]
const HUGGING_FACE_ENDPOINT_ENV: &str = "HF_ENDPOINT";
#[cfg(feature = "local-onnx")]
pub(super) const AUTO_EVALUATED_DEFAULT_ARTIFACT_SHA256: &str =
    "3970612d6f31b81d1dc30ddac0099da273b5753d1a07412e8390cf799e7836a6";
const MODEL_DOWNLOAD_LOCK_FILE: &str = ".remem-model-download.lock";
const MODEL_STATE_LOCK_FILE: &str = ".remem-model-state.lock";
const TOKENIZER_RUNTIME_FILES: &[&str] = &[
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];
#[derive(Debug)]
struct LocalEmbeddingModelUnavailableError(String);

impl std::fmt::Display for LocalEmbeddingModelUnavailableError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LocalEmbeddingModelUnavailableError {}

pub(super) fn is_model_unavailable_error(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<LocalEmbeddingModelUnavailableError>()
        .is_some()
}

pub(super) fn model_unavailable_error(reason: impl Into<String>) -> anyhow::Error {
    LocalEmbeddingModelUnavailableError(reason.into()).into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalEmbeddingInputKind {
    Query,
    Passage,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum LocalEmbeddingPreset {
    MultilingualE5Small,
    BgeM3,
}

impl LocalEmbeddingPreset {
    fn all() -> &'static [Self] {
        &[Self::MultilingualE5Small, Self::BgeM3]
    }

    fn default() -> Self {
        Self::MultilingualE5Small
    }

    fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" => Ok(Self::default()),
            "multilingual-e5-small"
            | "intfloat/multilingual-e5-small"
            | DEFAULT_LOCAL_SEMANTIC_MODEL => Ok(Self::MultilingualE5Small),
            "bge-m3" | "baai/bge-m3" | "fastembed-bge-m3-v1" => Ok(Self::BgeM3),
            other => bail!(
                "unsupported local embedding model preset {other}; supported presets: multilingual-e5-small, bge-m3"
            ),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::MultilingualE5Small => "multilingual-e5-small",
            Self::BgeM3 => "bge-m3",
        }
    }

    fn model_id(self) -> &'static str {
        match self {
            Self::MultilingualE5Small => DEFAULT_LOCAL_SEMANTIC_MODEL,
            Self::BgeM3 => "fastembed-bge-m3-v1",
        }
    }

    fn upstream_model(self) -> &'static str {
        match self {
            Self::MultilingualE5Small => "intfloat/multilingual-e5-small",
            Self::BgeM3 => "BAAI/bge-m3",
        }
    }

    fn source_url(self) -> String {
        format!("{HUGGING_FACE_BASE_URL}/{}", self.upstream_model())
    }

    fn dimensions(self) -> usize {
        match self {
            Self::MultilingualE5Small => DEFAULT_LOCAL_SEMANTIC_DIMENSIONS,
            Self::BgeM3 => 1024,
        }
    }

    fn cache_repo_dir(self) -> String {
        format!("models--{}", self.upstream_model()).replace('/', "--")
    }

    fn model_file(self) -> &'static str {
        "onnx/model.onnx"
    }

    fn additional_model_files(self) -> &'static [&'static str] {
        match self {
            Self::MultilingualE5Small => &[],
            Self::BgeM3 => &["onnx/model.onnx_data", "onnx/Constant_7_attr__value"],
        }
    }

    fn required_runtime_files(self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.model_file())
            .chain(self.additional_model_files().iter().copied())
            .chain(TOKENIZER_RUNTIME_FILES.iter().copied())
    }

    #[cfg(feature = "local-onnx")]
    fn prefix_input(self, text: &str, kind: LocalEmbeddingInputKind) -> String {
        match (self, kind) {
            (Self::MultilingualE5Small, LocalEmbeddingInputKind::Query) => {
                format!("query: {text}")
            }
            (Self::MultilingualE5Small, LocalEmbeddingInputKind::Passage) => {
                format!("passage: {text}")
            }
            _ => text.to_string(),
        }
    }

    #[cfg(feature = "local-onnx")]
    fn fastembed_model(self) -> fastembed::EmbeddingModel {
        match self {
            Self::MultilingualE5Small => fastembed::EmbeddingModel::MultilingualE5Small,
            Self::BgeM3 => fastembed::EmbeddingModel::BGEM3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalModelProfile {
    pub(super) model: String,
    pub(super) dimensions: usize,
    pub(super) install_dir: PathBuf,
    pub(super) artifact_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalEmbeddingDownloadReport {
    pub preset: String,
    pub model_id: String,
    pub upstream_model: String,
    pub dimensions: usize,
    pub install_dir: String,
    pub files_verified: usize,
    pub artifact_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalEmbeddingModelInventory {
    pub preset: String,
    pub model_id: String,
    pub upstream_model: String,
    pub dimensions: usize,
    pub install_dir: String,
    pub installed: bool,
    pub checksum_verified: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalEmbeddingInventoryReport {
    pub model_root: String,
    pub configured_preset: String,
    pub models: Vec<LocalEmbeddingModelInventory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalModelManifest {
    schema_version: u32,
    preset: String,
    model_id: String,
    upstream_model: String,
    dimensions: usize,
    runtime: String,
    source_url: Option<String>,
    downloaded_at_epoch: i64,
    files: Vec<LocalModelFile>,
    #[serde(default)]
    symlinks: Vec<LocalModelSymlink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalModelFile {
    path: String,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_sha256: Option<String>,
    bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalModelSymlink {
    path: String,
    link_target: String,
    resolved_path: String,
}

pub(super) fn model_root(config: &EmbeddingConfig) -> PathBuf {
    config
        .model_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::db::data_dir().join("models"))
}

#[cfg(feature = "local-onnx")]
pub(super) fn installed_model_profile(config: &EmbeddingConfig) -> Result<LocalModelProfile> {
    let preset = configured_local_preset_or_default(config)?;
    verified_profile_for_preset(config, preset)
}

#[cfg(feature = "local-onnx")]
pub(crate) fn with_configured_model_read_lock<T>(
    config: &EmbeddingConfig,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let preset = configured_local_preset_or_default(config)?;
    #[cfg(windows)]
    let _windows_install = windows_model_root::open_managed_install(config, preset, false)?
        .ok_or_else(|| windows_model_root::missing_install_error())?;
    #[cfg(windows)]
    let install_dir = _windows_install.install_dir().to_path_buf();
    #[cfg(not(windows))]
    let install_dir = install_dir_for_preset(config, preset);
    with_model_read_lock(&install_dir, operation)
}

#[cfg(not(feature = "local-onnx"))]
pub(super) fn installed_model_profile(config: &EmbeddingConfig) -> Result<LocalModelProfile> {
    let preset = configured_local_preset_or_default(config)?;
    #[cfg(windows)]
    windows_model_root::checked_model_root(config)?;
    Err(model_unavailable_error(format!(
        "local semantic embedding runtime is not built; rebuild remem with the local-onnx feature to use {}",
        preset.label()
    )))
}

#[cfg(not(feature = "local-onnx"))]
pub(crate) fn with_configured_model_read_lock<T>(
    config: &EmbeddingConfig,
    _operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let _ = installed_model_profile(config)?;
    Err(model_unavailable_error(
        "local semantic embedding runtime is not built",
    ))
}

pub(super) fn auto_installed_model_profile(
    config: &EmbeddingConfig,
) -> Result<Option<LocalModelProfile>> {
    let preset = configured_local_preset_or_default(config)?;
    #[cfg(windows)]
    let _windows_install = match windows_model_root::open_managed_install(config, preset, true)? {
        Some(install) => install,
        None => return Ok(None),
    };
    #[cfg(windows)]
    return auto_verified_model_profile(config, preset).map(Some);
    #[cfg(not(windows))]
    let install_dir = install_dir_for_preset(config, preset);
    #[cfg(not(windows))]
    match std::fs::symlink_metadata(&install_dir) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(metadata) if metadata.file_type().is_symlink() => Err(model_unavailable_error(format!(
            "local embedding model install path is a symlink: {}",
            install_dir.display()
        ))),
        Ok(metadata) if !metadata.file_type().is_dir() => Err(model_unavailable_error(format!(
            "local embedding model install path is not a directory: {}",
            install_dir.display()
        ))),
        Ok(_) => auto_verified_model_profile(config, preset).map(Some),
        Err(error) => Err(model_unavailable_error(format!(
            "inspect local embedding model {} in {}: {error:#}",
            preset.label(),
            install_dir.display()
        ))),
    }
}

pub(super) fn download_model(model: Option<&str>) -> Result<LocalEmbeddingDownloadReport> {
    let config = super::resolve_embedding_config()?;
    let preset = match model {
        Some(raw) => LocalEmbeddingPreset::parse(raw)?,
        None => configured_local_preset_or_default(&config)?,
    };

    #[cfg(not(feature = "local-onnx"))]
    {
        #[cfg(windows)]
        windows_model_root::checked_model_root(&config)?;
        bail!(
            "local semantic embedding runtime is not built; rebuild remem with the local-onnx feature to download {}",
            preset.label()
        );
    }

    #[cfg(feature = "local-onnx")]
    {
        #[cfg(windows)]
        let _windows_install = windows_model_root::create_managed_install(&config, preset)?;
        #[cfg(windows)]
        let install_dir = _windows_install.install_dir().to_path_buf();
        #[cfg(not(windows))]
        let install_dir = install_dir_for_preset(&config, preset);
        #[cfg(not(windows))]
        std::fs::create_dir_all(&install_dir).with_context(|| {
            format!("create local embedding model dir {}", install_dir.display())
        })?;
        let (lock_path, download_lock) =
            open_or_create_model_lock(&install_dir, MODEL_DOWNLOAD_LOCK_FILE)
                .context("open local model download serialization lock")?;
        fs2::FileExt::lock_exclusive(&download_lock)
            .with_context(|| format!("lock local model download {}", lock_path.display()))?;
        let (_state_lock_path, state_lock_file) =
            open_or_create_model_lock(&install_dir, MODEL_STATE_LOCK_FILE)
                .context("initialize local model state lock")?;
        drop(state_lock_file);
        let staging = download::materialize_hugging_face_artifacts(preset, &install_dir)?;
        let candidate = (|| {
            let prepared = download::prepare_downloaded_model(
                preset,
                staging.path(),
                chrono::Utc::now().timestamp(),
            )?;
            let imported = download::import_immutable_candidate(
                staging.path(),
                &install_dir,
                preset,
                &prepared.manifest,
            )?;
            Ok((prepared, imported))
        })();
        let (prepared, imported) = match candidate {
            Ok(candidate) => candidate,
            Err(error) => return download::cleanup_staging_after_error(staging, error),
        };
        staging.cleanup()?;
        let (state_lock_path, state_lock) =
            open_or_create_model_lock(&install_dir, MODEL_STATE_LOCK_FILE)
                .context("open local model state lock for publish")?;
        fs2::FileExt::lock_exclusive(&state_lock).with_context(|| {
            format!(
                "lock local model state for publish {}",
                state_lock_path.display()
            )
        })?;
        let verified = download::activate_candidate_manifest(
            &install_dir,
            preset,
            prepared.manifest,
            prepared.artifact_sha256,
            imported,
        )?;
        let artifact_sha256 = verified.artifact_sha256;
        let manifest = verified.manifest;
        Ok(LocalEmbeddingDownloadReport {
            preset: manifest.preset,
            model_id: manifest.model_id,
            upstream_model: manifest.upstream_model,
            dimensions: manifest.dimensions,
            install_dir: install_dir.display().to_string(),
            files_verified: manifest.files.len(),
            artifact_sha256,
        })
    }
}

pub(super) fn inventory() -> Result<LocalEmbeddingInventoryReport> {
    let config = super::resolve_embedding_config()?;
    #[cfg(windows)]
    let root = windows_model_root::checked_model_root(&config)?;
    #[cfg(not(windows))]
    let root = model_root(&config);
    let configured = configured_local_preset_or_default(&config)?;
    let models = LocalEmbeddingPreset::all()
        .iter()
        .copied()
        .map(|preset| inventory_for_preset(&config, preset))
        .collect::<Result<Vec<_>>>()?;
    Ok(LocalEmbeddingInventoryReport {
        model_root: root.display().to_string(),
        configured_preset: configured.label().to_string(),
        models,
    })
}

#[cfg(feature = "local-onnx")]
pub(super) fn embed_text(
    text: &str,
    config: &EmbeddingConfig,
    kind: LocalEmbeddingInputKind,
) -> Result<TextEmbedding> {
    let preset = configured_local_preset_or_default(config)?;
    #[cfg(windows)]
    let _windows_install = windows_model_root::open_managed_install(config, preset, false)?
        .ok_or_else(|| windows_model_root::missing_install_error())?;
    #[cfg(windows)]
    let install_dir = _windows_install.install_dir().to_path_buf();
    #[cfg(not(windows))]
    let install_dir = install_dir_for_preset(config, preset);
    #[cfg(test)]
    if let Some(failure) = test_support::take_next_embed_failure(&install_dir)? {
        return match failure {
            test_support::TestEmbedFailure::ModelUnavailable(reason) => {
                Err(model_unavailable_error(reason))
            }
            test_support::TestEmbedFailure::Generic(reason) => Err(anyhow::anyhow!(reason)),
        };
    }
    read_verified_manifest_compatible(&install_dir, Some(preset)).map_err(|error| {
        model_unavailable_error(format!(
            "local embedding model {} is not ready in {}: {error:#}",
            preset.label(),
            install_dir.display()
        ))
    })?;
    with_model_read_lock(&install_dir, || {
        let verified =
            read_verified_manifest_unlocked(&install_dir, Some(preset)).map_err(|error| {
                model_unavailable_error(format!(
                    "local embedding model {} is not ready in {}: {error:#}",
                    preset.label(),
                    install_dir.display()
                ))
            })?;
        if config.provider == super::EmbeddingProvider::Auto {
            require_auto_evaluated_artifact(&install_dir, preset, &verified.artifact_sha256)?;
        }
        let profile = profile_from_verified_manifest(&install_dir, &verified);
        let values = runtime::embed_with_verified_model(
            preset,
            &install_dir,
            &verified.manifest,
            &profile.artifact_sha256,
            text,
            kind,
        )?;
        if values.len() != profile.dimensions {
            bail!(
                "local embedding model {} returned {} dimensions, expected {}",
                profile.model,
                values.len(),
                profile.dimensions
            );
        }
        TextEmbedding::new(profile.model, values)
    })
}

#[cfg(not(feature = "local-onnx"))]
pub(super) fn embed_text(
    _text: &str,
    config: &EmbeddingConfig,
    _kind: LocalEmbeddingInputKind,
) -> Result<TextEmbedding> {
    let preset = configured_local_preset_or_default(config)?;
    #[cfg(windows)]
    windows_model_root::checked_model_root(config)?;
    Err(model_unavailable_error(format!(
        "local semantic embedding runtime is not built; rebuild remem with the local-onnx feature to use {}",
        preset.label()
    )))
}

fn configured_preset(config: &EmbeddingConfig) -> Result<LocalEmbeddingPreset> {
    let raw = config.model.trim();
    if raw.is_empty() || raw == super::OPENAI_DEFAULT_MODEL {
        return Ok(LocalEmbeddingPreset::default());
    }
    LocalEmbeddingPreset::parse(raw)
}

pub(super) fn configured_model_id(config: &EmbeddingConfig) -> Result<String> {
    Ok(configured_preset(config)?.model_id().to_string())
}

fn configured_local_preset_or_default(config: &EmbeddingConfig) -> Result<LocalEmbeddingPreset> {
    if config.provider == super::EmbeddingProvider::Local {
        configured_preset(config)
    } else {
        Ok(LocalEmbeddingPreset::default())
    }
}

#[cfg(feature = "local-onnx")]
fn verified_profile_for_preset(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
) -> Result<LocalModelProfile> {
    verified_profile_for_preset_with_policy(config, preset, false)
}

#[cfg(feature = "local-onnx")]
fn auto_verified_model_profile(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
) -> Result<LocalModelProfile> {
    verified_profile_for_preset_with_policy(config, preset, true)
}

#[cfg(not(feature = "local-onnx"))]
fn auto_verified_model_profile(
    config: &EmbeddingConfig,
    _preset: LocalEmbeddingPreset,
) -> Result<LocalModelProfile> {
    installed_model_profile(config)
}

#[cfg(feature = "local-onnx")]
fn verified_profile_for_preset_with_policy(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
    enforce_auto_evaluated_artifact: bool,
) -> Result<LocalModelProfile> {
    #[cfg(windows)]
    let _windows_install = windows_model_root::open_managed_install(config, preset, false)?
        .ok_or_else(|| windows_model_root::missing_install_error())?;
    #[cfg(windows)]
    let install_dir = _windows_install.install_dir().to_path_buf();
    #[cfg(not(windows))]
    let install_dir = install_dir_for_preset(config, preset);
    read_verified_manifest_compatible(&install_dir, Some(preset)).map_err(|error| {
        model_unavailable_error(format!(
            "local embedding model {} is not ready in {}: {error:#}",
            preset.label(),
            install_dir.display()
        ))
    })?;
    with_model_read_lock(&install_dir, || {
        let verified = read_verified_manifest_unlocked(&install_dir, Some(preset))?;
        if enforce_auto_evaluated_artifact {
            require_auto_evaluated_artifact(&install_dir, preset, &verified.artifact_sha256)?;
        }
        runtime::ensure_verified_model_ready(
            preset,
            &install_dir,
            &verified.manifest,
            &verified.artifact_sha256,
        )?;
        Ok(profile_from_verified_manifest(&install_dir, &verified))
    })
    .map_err(|error| {
        model_unavailable_error(format!(
            "local embedding model {} is not ready in {}: {error:#}",
            preset.label(),
            install_dir.display()
        ))
    })
}

#[cfg(feature = "local-onnx")]
fn auto_artifact_is_trusted(_install_dir: &Path, artifact_sha256: &str) -> Result<bool> {
    if artifact_sha256 == AUTO_EVALUATED_DEFAULT_ARTIFACT_SHA256 {
        return Ok(true);
    }
    #[cfg(test)]
    if test_support::is_test_auto_artifact_trusted(_install_dir)? {
        return Ok(true);
    }
    Ok(false)
}

#[cfg(feature = "local-onnx")]
fn require_auto_evaluated_artifact(
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
    artifact_sha256: &str,
) -> Result<()> {
    if auto_artifact_is_trusted(install_dir, artifact_sha256)? {
        return Ok(());
    }
    Err(model_unavailable_error(format!(
        "automatic local embedding requires evaluated {} artifact sha256:{}; installed artifact sha256:{} is not trusted for Auto. Upgrade remem or redownload the model from {}",
        preset.label(),
        AUTO_EVALUATED_DEFAULT_ARTIFACT_SHA256,
        artifact_sha256,
        HUGGING_FACE_BASE_URL
    )))
}

#[cfg(feature = "local-onnx")]
fn profile_from_verified_manifest(
    install_dir: &Path,
    verified: &manifest::VerifiedLocalManifest,
) -> LocalModelProfile {
    LocalModelProfile {
        model: format!(
            "{}@sha256:{}",
            verified.manifest.model_id, verified.artifact_sha256
        ),
        dimensions: verified.manifest.dimensions,
        install_dir: install_dir.to_path_buf(),
        artifact_sha256: verified.artifact_sha256.clone(),
    }
}

fn inventory_for_preset(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
) -> Result<LocalEmbeddingModelInventory> {
    #[cfg(windows)]
    let _windows_install = match windows_model_root::open_managed_install(config, preset, true)? {
        Some(install) => install,
        None => {
            return Ok(LocalEmbeddingModelInventory {
                preset: preset.label().to_string(),
                model_id: preset.model_id().to_string(),
                upstream_model: preset.upstream_model().to_string(),
                dimensions: preset.dimensions(),
                install_dir: model_root(config)
                    .join(preset.model_id())
                    .display()
                    .to_string(),
                installed: false,
                checksum_verified: false,
                unavailable_reason: Some("local embedding model is not installed".to_string()),
            });
        }
    };
    #[cfg(windows)]
    let install_dir = _windows_install.install_dir().to_path_buf();
    #[cfg(not(windows))]
    let install_dir = install_dir_for_preset(config, preset);
    match read_verified_manifest_compatible(&install_dir, Some(preset)) {
        Ok(verified) => Ok(LocalEmbeddingModelInventory {
            preset: verified.manifest.preset,
            model_id: verified.manifest.model_id,
            upstream_model: verified.manifest.upstream_model,
            dimensions: verified.manifest.dimensions,
            install_dir: install_dir.display().to_string(),
            installed: true,
            checksum_verified: is_sha256_hex(&verified.artifact_sha256),
            unavailable_reason: None,
        }),
        Err(error) => Ok(LocalEmbeddingModelInventory {
            preset: preset.label().to_string(),
            model_id: preset.model_id().to_string(),
            upstream_model: preset.upstream_model().to_string(),
            dimensions: preset.dimensions(),
            install_dir: install_dir.display().to_string(),
            installed: false,
            checksum_verified: false,
            unavailable_reason: Some(error.to_string()),
        }),
    }
}

#[cfg(not(windows))]
fn install_dir_for_preset(config: &EmbeddingConfig, preset: LocalEmbeddingPreset) -> PathBuf {
    model_root(config).join(preset.model_id())
}

fn checked_relative_path(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        bail!("manifest path must be relative: {raw}");
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("manifest path must not contain parent/current components: {raw}");
    }
    Ok(path)
}

fn source_sha256_from_hf_blob_path(relative: &str, actual_sha256: &str) -> Result<Option<String>> {
    let parts = relative.split('/').collect::<Vec<_>>();
    let Some(file_name) = parts.last().copied() else {
        return Ok(None);
    };
    if parts.len() < 2 || parts[parts.len() - 2] != "blobs" || !is_sha256_hex(file_name) {
        return Ok(None);
    }
    if file_name != actual_sha256 {
        bail!(
            "source checksum mismatch for Hugging Face cache blob {relative}: expected {file_name}, got {actual_sha256}"
        );
    }
    Ok(Some(file_name.to_string()))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_file(path: &Path) -> Result<String> {
    #[cfg(test)]
    let pending_hash = hash_counter::PendingModelFileHash::for_path(path)?;
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)
            .with_context(|| format!("read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    #[cfg(test)]
    pending_hash.record()?;
    Ok(sha256)
}

#[cfg(test)]
mod tests;
