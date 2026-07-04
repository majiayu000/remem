#[cfg(feature = "local-onnx")]
use std::cell::RefCell;
#[cfg(feature = "local-onnx")]
use std::collections::{hash_map::Entry, HashMap};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{EmbeddingConfig, TextEmbedding};

pub(super) const DEFAULT_LOCAL_SEMANTIC_DIMENSIONS: usize = 384;
pub(super) const DEFAULT_LOCAL_SEMANTIC_MODEL: &str = "fastembed-intfloat-multilingual-e5-small-v1";

const MANIFEST_FILE: &str = "remem-model-manifest.json";
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const FASTEMBED_RUNTIME: &str = "fastembed-rs/onnxruntime";
const HUGGING_FACE_BASE_URL: &str = "https://huggingface.co";

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

#[cfg(feature = "local-onnx")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LocalModelCacheKey {
    preset: LocalEmbeddingPreset,
    install_dir: PathBuf,
}

#[cfg(feature = "local-onnx")]
thread_local! {
    static LOCAL_MODEL_CACHE: RefCell<HashMap<LocalModelCacheKey, fastembed::TextEmbedding>> =
        RefCell::new(HashMap::new());
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalEmbeddingDownloadReport {
    pub preset: String,
    pub model_id: String,
    pub upstream_model: String,
    pub dimensions: usize,
    pub install_dir: String,
    pub files_verified: usize,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LocalModelFile {
    path: String,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_sha256: Option<String>,
    bytes: u64,
}

pub(super) fn model_root(config: &EmbeddingConfig) -> PathBuf {
    config
        .model_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::db::data_dir().join("models"))
}

pub(super) fn installed_model_profile(config: &EmbeddingConfig) -> Result<LocalModelProfile> {
    let preset = configured_preset(config)?;
    verified_profile_for_preset(config, preset)
}

pub(super) fn download_model(model: Option<&str>) -> Result<LocalEmbeddingDownloadReport> {
    let config = super::resolve_embedding_config()?;
    let preset = match model {
        Some(raw) => LocalEmbeddingPreset::parse(raw)?,
        None => configured_local_preset_or_default(&config)?,
    };
    let install_dir = install_dir_for_preset(&config, preset);
    std::fs::create_dir_all(&install_dir)
        .with_context(|| format!("create local embedding model dir {}", install_dir.display()))?;
    materialize_fastembed_model(preset, &install_dir)?;
    let files = collect_model_files(&install_dir)?;
    if files.is_empty() {
        bail!(
            "local embedding download did not materialize model files in {}",
            install_dir.display()
        );
    }
    let manifest = LocalModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        dimensions: preset.dimensions(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        source_url: Some(preset.source_url()),
        downloaded_at_epoch: chrono::Utc::now().timestamp(),
        files,
    };
    write_manifest(&install_dir, &manifest)?;
    let verified = read_verified_manifest(&install_dir, Some(preset))?;
    Ok(LocalEmbeddingDownloadReport {
        preset: verified.preset,
        model_id: verified.model_id,
        upstream_model: verified.upstream_model,
        dimensions: verified.dimensions,
        install_dir: install_dir.display().to_string(),
        files_verified: verified.files.len(),
    })
}

pub(super) fn inventory() -> Result<LocalEmbeddingInventoryReport> {
    let config = super::resolve_embedding_config()?;
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

pub(super) fn embed_text(
    text: &str,
    config: &EmbeddingConfig,
    kind: LocalEmbeddingInputKind,
) -> Result<TextEmbedding> {
    let preset = configured_preset(config)?;
    let profile = verified_profile_for_preset(config, preset)?;
    let values = embed_with_fastembed(preset, &profile.install_dir, text, kind)?;
    if values.len() != profile.dimensions {
        bail!(
            "local embedding model {} returned {} dimensions, expected {}",
            profile.model,
            values.len(),
            profile.dimensions
        );
    }
    TextEmbedding::new(profile.model, values)
}

fn configured_preset(config: &EmbeddingConfig) -> Result<LocalEmbeddingPreset> {
    let raw = config.model.trim();
    if raw.is_empty() || raw == super::OPENAI_DEFAULT_MODEL {
        return Ok(LocalEmbeddingPreset::default());
    }
    LocalEmbeddingPreset::parse(raw)
}

fn configured_local_preset_or_default(config: &EmbeddingConfig) -> Result<LocalEmbeddingPreset> {
    if config.provider == super::EmbeddingProvider::Local {
        configured_preset(config)
    } else {
        Ok(LocalEmbeddingPreset::default())
    }
}

fn verified_profile_for_preset(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
) -> Result<LocalModelProfile> {
    let install_dir = install_dir_for_preset(config, preset);
    let manifest = read_verified_manifest(&install_dir, Some(preset)).map_err(|error| {
        model_unavailable_error(format!(
            "local embedding model {} is not ready in {}: {error}",
            preset.label(),
            install_dir.display()
        ))
    })?;
    Ok(LocalModelProfile {
        model: manifest.model_id,
        dimensions: manifest.dimensions,
        install_dir,
    })
}

fn inventory_for_preset(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
) -> Result<LocalEmbeddingModelInventory> {
    let install_dir = install_dir_for_preset(config, preset);
    match read_verified_manifest(&install_dir, Some(preset)) {
        Ok(_) => Ok(LocalEmbeddingModelInventory {
            preset: preset.label().to_string(),
            model_id: preset.model_id().to_string(),
            upstream_model: preset.upstream_model().to_string(),
            dimensions: preset.dimensions(),
            install_dir: install_dir.display().to_string(),
            installed: true,
            checksum_verified: true,
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

fn install_dir_for_preset(config: &EmbeddingConfig, preset: LocalEmbeddingPreset) -> PathBuf {
    model_root(config).join(preset.model_id())
}

#[cfg(feature = "local-onnx")]
fn materialize_fastembed_model(preset: LocalEmbeddingPreset, install_dir: &Path) -> Result<()> {
    let options = fastembed::TextInitOptions::new(preset.fastembed_model())
        .with_cache_dir(install_dir.to_path_buf())
        .with_show_download_progress(true);
    let mut model = fastembed::TextEmbedding::try_new(options)
        .with_context(|| format!("initialize local embedding model {}", preset.label()))?;
    let probe = preset.prefix_input(
        "remem local embedding readiness probe",
        LocalEmbeddingInputKind::Generic,
    );
    let embeddings = model
        .embed([probe.as_str()], Some(1))
        .with_context(|| format!("probe local embedding model {}", preset.label()))?;
    if embeddings.len() != 1 {
        bail!(
            "local embedding model {} returned {} probe embeddings",
            preset.label(),
            embeddings.len()
        );
    }
    Ok(())
}

#[cfg(not(feature = "local-onnx"))]
fn materialize_fastembed_model(preset: LocalEmbeddingPreset, _install_dir: &Path) -> Result<()> {
    bail!(
        "local semantic embedding runtime is not built; rebuild remem with the local-onnx feature to download {}",
        preset.label()
    )
}

#[cfg(feature = "local-onnx")]
fn embed_with_fastembed(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    text: &str,
    kind: LocalEmbeddingInputKind,
) -> Result<Vec<f32>> {
    let input = preset.prefix_input(text, kind);
    let key = LocalModelCacheKey {
        preset,
        install_dir: install_dir.to_path_buf(),
    };
    LOCAL_MODEL_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let model = match cache.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let options = fastembed::TextInitOptions::new(preset.fastembed_model())
                    .with_cache_dir(install_dir.to_path_buf())
                    .with_show_download_progress(false);
                let model = fastembed::TextEmbedding::try_new(options).with_context(|| {
                    format!("initialize local embedding model {}", preset.label())
                })?;
                entry.insert(model)
            }
        };
        let mut embeddings = model
            .embed([input.as_str()], Some(1))
            .with_context(|| format!("embed text with local model {}", preset.label()))?;
        let first = embeddings
            .pop()
            .context("local embedding model did not return an embedding")?;
        if !embeddings.is_empty() {
            bail!("local embedding model returned multiple embeddings for single input");
        }
        Ok(first)
    })
}

#[cfg(not(feature = "local-onnx"))]
fn embed_with_fastembed(
    preset: LocalEmbeddingPreset,
    _install_dir: &Path,
    _text: &str,
    _kind: LocalEmbeddingInputKind,
) -> Result<Vec<f32>> {
    Err(model_unavailable_error(format!(
        "local semantic embedding runtime is not built; rebuild remem with the local-onnx feature to use {}",
        preset.label()
    )))
}

fn read_verified_manifest(
    install_dir: &Path,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<LocalModelManifest> {
    let path = install_dir.join(MANIFEST_FILE);
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let manifest: LocalModelManifest =
        serde_json::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
    verify_manifest_header(&manifest, expected_preset)?;
    for file in &manifest.files {
        verify_manifest_file(install_dir, file)?;
    }
    Ok(manifest)
}

fn verify_manifest_header(
    manifest: &LocalModelManifest,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<()> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        bail!(
            "unsupported manifest schema {}, expected {}",
            manifest.schema_version,
            MANIFEST_SCHEMA_VERSION
        );
    }
    let preset = LocalEmbeddingPreset::parse(&manifest.preset)?;
    if let Some(expected) = expected_preset {
        if preset != expected {
            bail!(
                "manifest preset {} does not match expected {}",
                manifest.preset,
                expected.label()
            );
        }
    }
    if manifest.model_id != preset.model_id() {
        bail!(
            "manifest model_id {} does not match preset {}",
            manifest.model_id,
            preset.model_id()
        );
    }
    if manifest.dimensions != preset.dimensions() {
        bail!(
            "manifest dimensions {} do not match preset {} dimensions {}",
            manifest.dimensions,
            preset.label(),
            preset.dimensions()
        );
    }
    if manifest.runtime != FASTEMBED_RUNTIME {
        bail!("unsupported local embedding runtime {}", manifest.runtime);
    }
    if let Some(source_url) = manifest.source_url.as_deref() {
        let expected = preset.source_url();
        if source_url != expected {
            bail!(
                "manifest source_url {} does not match preset {} source {}",
                source_url,
                preset.label(),
                expected
            );
        }
    }
    if manifest.files.is_empty() {
        bail!("local embedding manifest has no verified files");
    }
    Ok(())
}

fn verify_manifest_file(install_dir: &Path, file: &LocalModelFile) -> Result<()> {
    let relative = checked_relative_path(&file.path)?;
    let path = install_dir.join(relative);
    let metadata = std::fs::metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    if !metadata.is_file() {
        bail!("manifest path is not a file: {}", path.display());
    }
    if metadata.len() != file.bytes {
        bail!(
            "checksum target {} size changed: expected {} bytes, got {}",
            path.display(),
            file.bytes,
            metadata.len()
        );
    }
    let actual = sha256_file(&path)?;
    if actual != file.sha256 {
        bail!(
            "checksum mismatch for {}: expected {}, got {}",
            path.display(),
            file.sha256,
            actual
        );
    }
    if let Some(source_sha256) = file.source_sha256.as_deref() {
        if actual != source_sha256 {
            bail!(
                "source checksum mismatch for {}: expected {}, got {}",
                path.display(),
                source_sha256,
                actual
            );
        }
    }
    Ok(())
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

fn write_manifest(install_dir: &Path, manifest: &LocalModelManifest) -> Result<()> {
    let path = install_dir.join(MANIFEST_FILE);
    let tmp = install_dir.join(format!("{MANIFEST_FILE}.tmp"));
    let content = serde_json::to_vec_pretty(manifest).context("serialize local model manifest")?;
    std::fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("replace local model manifest {}", path.display()))?;
    Ok(())
}

fn collect_model_files(root: &Path) -> Result<Vec<LocalModelFile>> {
    let mut files = Vec::new();
    collect_model_files_inner(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_model_files_inner(
    root: &Path,
    current: &Path,
    files: &mut Vec<LocalModelFile>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(current).with_context(|| format!("read {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name == MANIFEST_FILE || file_name == format!("{MANIFEST_FILE}.tmp") {
            continue;
        }
        if file_name == ".locks" || file_name.ends_with(".lock") || file_name.ends_with(".tmp") {
            continue;
        }
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_model_files_inner(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path.strip_prefix(root).with_context(|| {
                format!("make {} relative to {}", path.display(), root.display())
            })?;
            let relative = relative
                .components()
                .map(|component| match component {
                    Component::Normal(value) => Ok(value.to_string_lossy().to_string()),
                    _ => bail!("unexpected non-normal cache path {}", path.display()),
                })
                .collect::<Result<Vec<_>>>()?
                .join("/");
            let sha256 = sha256_file(&path)?;
            let source_sha256 = source_sha256_from_hf_blob_path(&relative, &sha256)?;
            files.push(LocalModelFile {
                path: relative,
                sha256,
                source_sha256,
                bytes: metadata.len(),
            });
        }
    }
    Ok(())
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
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hf_cache_blob_source_sha_is_verified() -> Result<()> {
        let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        let verified = source_sha256_from_hf_blob_path(&format!("models--demo/blobs/{sha}"), sha)?;

        assert_eq!(verified.as_deref(), Some(sha));
        Ok(())
    }

    #[test]
    fn hf_cache_blob_source_sha_mismatch_fails() {
        let source = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let actual = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

        let error =
            source_sha256_from_hf_blob_path(&format!("models--demo/blobs/{source}"), actual)
                .unwrap_err();

        assert!(error.to_string().contains("source checksum mismatch"));
    }
}
