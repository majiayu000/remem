use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::RerankConfig;
use super::types::RerankDisabledReason;

const MANIFEST_FILE: &str = "remem-reranker-manifest.json";
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const MANIFEST_KIND: &str = "reranker";
const FASTEMBED_RUNTIME: &str = "fastembed-rs/onnxruntime";
const HUGGING_FACE_BASE_URL: &str = "https://huggingface.co";

/// Closed set of supported local reranker presets. The reranker owns its own
/// model kind and inventory; embedding manifests are never accepted as
/// reranker evidence.
///
/// Implementation proposal (maintainer-adjustable): `bge-reranker-base` is the
/// most conservative locally runnable cross-encoder available in the pinned
/// fastembed 5.17.2 dependency (English + Chinese, no external-data ONNX
/// files).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RerankerPreset {
    BgeRerankerBase,
}

impl RerankerPreset {
    pub fn all() -> &'static [Self] {
        &[Self::BgeRerankerBase]
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "bge-reranker-base" | "baai/bge-reranker-base" | DEFAULT_RERANKER_MODEL_ID => {
                Ok(Self::BgeRerankerBase)
            }
            other => {
                bail!("unsupported reranker preset {other}; supported presets: bge-reranker-base")
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::BgeRerankerBase => "bge-reranker-base",
        }
    }

    pub fn model_id(self) -> &'static str {
        match self {
            Self::BgeRerankerBase => DEFAULT_RERANKER_MODEL_ID,
        }
    }

    pub fn upstream_model(self) -> &'static str {
        match self {
            Self::BgeRerankerBase => "BAAI/bge-reranker-base",
        }
    }

    fn source_url(self) -> String {
        format!("{HUGGING_FACE_BASE_URL}/{}", self.upstream_model())
    }

    #[cfg(feature = "local-onnx")]
    pub(super) fn fastembed_model(self) -> fastembed::RerankerModel {
        match self {
            Self::BgeRerankerBase => fastembed::RerankerModel::BGERerankerBase,
        }
    }
}

const DEFAULT_RERANKER_MODEL_ID: &str = "fastembed-bge-reranker-base-v1";

/// Relative paths of the files the local runtime loads. Every role file must
/// also appear in the verified `files` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankerRoleFiles {
    pub onnx_file: String,
    pub tokenizer_file: String,
    pub config_file: String,
    pub special_tokens_map_file: String,
    pub tokenizer_config_file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RerankerManifest {
    schema_version: u32,
    kind: String,
    pub preset: String,
    pub model_id: String,
    pub upstream_model: String,
    runtime: String,
    source_url: Option<String>,
    downloaded_at_epoch: i64,
    pub roles: RerankerRoleFiles,
    files: Vec<RerankerModelFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RerankerModelFile {
    path: String,
    sha256: String,
    bytes: u64,
}

/// A fully verified local reranker model the runtime may load. Verification
/// (bytes + SHA-256 for every manifest file) happens before this value exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRerankerModel {
    pub preset: RerankerPreset,
    pub install_dir: PathBuf,
    pub manifest_sha256: String,
    pub manifest: RerankerManifest,
}

/// Inventory state distinguishing intentional-off from broken states so the
/// stage and doctor can report a stable closed reason.
#[derive(Debug, Clone, PartialEq)]
pub enum RerankerInventoryState {
    Ready(Box<VerifiedRerankerModel>),
    Missing(String),
    Corrupt(String),
}

impl RerankerInventoryState {
    pub fn disabled_reason(&self) -> Option<RerankDisabledReason> {
        match self {
            Self::Ready(_) => None,
            Self::Missing(_) => Some(RerankDisabledReason::ModelMissing),
            Self::Corrupt(_) => Some(RerankDisabledReason::ModelCorrupt),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RerankerDownloadReport {
    pub preset: String,
    pub model_id: String,
    pub upstream_model: String,
    pub install_dir: String,
    pub files_verified: usize,
    pub manifest_sha256: String,
}

pub fn model_root(config: &RerankConfig) -> PathBuf {
    config
        .model_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::db::data_dir().join("models"))
}

pub fn install_dir_for_preset(config: &RerankConfig, preset: RerankerPreset) -> PathBuf {
    model_root(config).join(preset.model_id())
}

/// Inspect the configured preset's local inventory without touching the
/// network. Never downloads.
pub fn inventory_state(config: &RerankConfig) -> Result<RerankerInventoryState> {
    let preset = RerankerPreset::parse(&config.preset)?;
    let install_dir = install_dir_for_preset(config, preset);
    let manifest_path = install_dir.join(MANIFEST_FILE);
    let content = match std::fs::read(&manifest_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RerankerInventoryState::Missing(format!(
                "reranker manifest not installed at {}; run `remem reranker download`",
                manifest_path.display()
            )));
        }
        Err(error) => {
            return Ok(RerankerInventoryState::Corrupt(format!(
                "read {}: {error}",
                manifest_path.display()
            )));
        }
    };
    let manifest: RerankerManifest = match serde_json::from_slice(&content) {
        Ok(manifest) => manifest,
        Err(error) => {
            return Ok(RerankerInventoryState::Corrupt(format!(
                "parse {}: {error}",
                manifest_path.display()
            )));
        }
    };
    if let Err(error) = verify_manifest(&install_dir, &manifest, preset) {
        return Ok(RerankerInventoryState::Corrupt(error.to_string()));
    }
    Ok(RerankerInventoryState::Ready(Box::new(
        VerifiedRerankerModel {
            preset,
            install_dir,
            manifest_sha256: sha256_hex(&content),
            manifest,
        },
    )))
}

/// Explicit, user-initiated model download. This is the only code path in the
/// reranker that may touch the network; search/API/MCP/SessionStart/doctor
/// only ever read the verified local inventory.
pub fn download_model(model: Option<&str>) -> Result<RerankerDownloadReport> {
    let config = super::config::resolve_rerank_config()?;
    let preset = match model {
        Some(raw) => RerankerPreset::parse(raw)?,
        None => RerankerPreset::parse(&config.preset)?,
    };
    let install_dir = install_dir_for_preset(&config, preset);
    std::fs::create_dir_all(&install_dir)
        .with_context(|| format!("create reranker model dir {}", install_dir.display()))?;
    materialize_fastembed_model(preset, &install_dir)?;
    let files = collect_model_files(&install_dir)?;
    if files.is_empty() {
        bail!(
            "reranker download did not materialize model files in {}",
            install_dir.display()
        );
    }
    let roles = detect_role_files(&files)?;
    let manifest = RerankerManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        kind: MANIFEST_KIND.to_string(),
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        source_url: Some(preset.source_url()),
        downloaded_at_epoch: chrono::Utc::now().timestamp(),
        roles,
        files,
    };
    write_manifest(&install_dir, &manifest)?;
    let verify_config = RerankConfig {
        preset: preset.label().to_string(),
        model_dir: config.model_dir.clone(),
        ..config
    };
    match inventory_state(&verify_config)? {
        RerankerInventoryState::Ready(verified) => Ok(RerankerDownloadReport {
            preset: verified.manifest.preset.clone(),
            model_id: verified.manifest.model_id.clone(),
            upstream_model: verified.manifest.upstream_model.clone(),
            install_dir: install_dir.display().to_string(),
            files_verified: verified.manifest.files.len(),
            manifest_sha256: verified.manifest_sha256.clone(),
        }),
        RerankerInventoryState::Missing(reason) | RerankerInventoryState::Corrupt(reason) => {
            bail!("reranker download verification failed: {reason}")
        }
    }
}

#[cfg(feature = "local-onnx")]
fn materialize_fastembed_model(preset: RerankerPreset, install_dir: &Path) -> Result<()> {
    let options = fastembed::RerankInitOptions::new(preset.fastembed_model())
        .with_cache_dir(install_dir.to_path_buf())
        .with_show_download_progress(true);
    let mut model = fastembed::TextRerank::try_new(options)
        .with_context(|| format!("initialize reranker model {}", preset.label()))?;
    let probe = model
        .rerank(
            "remem reranker readiness probe",
            ["probe document"],
            false,
            Some(1),
        )
        .with_context(|| format!("probe reranker model {}", preset.label()))?;
    if probe.len() != 1 {
        bail!(
            "reranker model {} returned {} probe results",
            preset.label(),
            probe.len()
        );
    }
    Ok(())
}

#[cfg(not(feature = "local-onnx"))]
fn materialize_fastembed_model(preset: RerankerPreset, _install_dir: &Path) -> Result<()> {
    bail!(
        "local reranker runtime is not built; rebuild remem with the local-onnx feature to download {}",
        preset.label()
    )
}

fn verify_manifest(
    install_dir: &Path,
    manifest: &RerankerManifest,
    expected_preset: RerankerPreset,
) -> Result<()> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        bail!(
            "unsupported reranker manifest schema {}, expected {}",
            manifest.schema_version,
            MANIFEST_SCHEMA_VERSION
        );
    }
    if manifest.kind != MANIFEST_KIND {
        bail!(
            "manifest kind {} is not a reranker manifest; embedding manifests are not reranker evidence",
            manifest.kind
        );
    }
    let preset = RerankerPreset::parse(&manifest.preset)?;
    if preset != expected_preset {
        bail!(
            "manifest preset {} does not match expected {}",
            manifest.preset,
            expected_preset.label()
        );
    }
    if manifest.model_id != preset.model_id() {
        bail!(
            "manifest model_id {} does not match preset {}",
            manifest.model_id,
            preset.model_id()
        );
    }
    if manifest.runtime != FASTEMBED_RUNTIME {
        bail!("unsupported reranker runtime {}", manifest.runtime);
    }
    if manifest.files.is_empty() {
        bail!("reranker manifest has no verified files");
    }
    for file in &manifest.files {
        verify_manifest_file(install_dir, file)?;
    }
    for role in [
        &manifest.roles.onnx_file,
        &manifest.roles.tokenizer_file,
        &manifest.roles.config_file,
        &manifest.roles.special_tokens_map_file,
        &manifest.roles.tokenizer_config_file,
    ] {
        if !manifest.files.iter().any(|file| &file.path == role) {
            bail!("manifest role file {role} is not in the verified file list");
        }
    }
    Ok(())
}

fn verify_manifest_file(install_dir: &Path, file: &RerankerModelFile) -> Result<()> {
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
    Ok(())
}

pub(super) fn role_path(install_dir: &Path, relative: &str) -> Result<PathBuf> {
    Ok(install_dir.join(checked_relative_path(relative)?))
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

fn write_manifest(install_dir: &Path, manifest: &RerankerManifest) -> Result<()> {
    let path = install_dir.join(MANIFEST_FILE);
    let tmp = install_dir.join(format!("{MANIFEST_FILE}.tmp"));
    let content = serde_json::to_vec_pretty(manifest).context("serialize reranker manifest")?;
    std::fs::write(&tmp, content).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("replace reranker manifest {}", path.display()))?;
    Ok(())
}

fn detect_role_files(files: &[RerankerModelFile]) -> Result<RerankerRoleFiles> {
    let find = |suffix: &str| -> Result<String> {
        let mut matches = files
            .iter()
            .filter(|file| file.path == suffix || file.path.ends_with(&format!("/{suffix}")))
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        matches.sort();
        matches
            .into_iter()
            .next()
            .with_context(|| format!("reranker download is missing required file {suffix}"))
    };
    Ok(RerankerRoleFiles {
        onnx_file: find("onnx/model.onnx").or_else(|_| find("model.onnx"))?,
        tokenizer_file: find("tokenizer.json")?,
        config_file: find("config.json")?,
        special_tokens_map_file: find("special_tokens_map.json")?,
        tokenizer_config_file: find("tokenizer_config.json")?,
    })
}

fn collect_model_files(root: &Path) -> Result<Vec<RerankerModelFile>> {
    let mut files = Vec::new();
    collect_model_files_inner(root, root, &mut files)?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_model_files_inner(
    root: &Path,
    current: &Path,
    files: &mut Vec<RerankerModelFile>,
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
            files.push(RerankerModelFile {
                path: relative,
                sha256: sha256_file(&path)?,
                bytes: metadata.len(),
            });
        }
    }
    Ok(())
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
    Ok(hex_digest(hasher))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher)
}

fn hex_digest(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
pub(super) fn write_test_manifest(
    install_dir: &Path,
    preset: RerankerPreset,
    file_contents: &[(&str, &[u8])],
) -> Result<()> {
    std::fs::create_dir_all(install_dir)?;
    for (relative, content) in file_contents {
        let path = install_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    let files = collect_model_files(install_dir)?;
    let roles = detect_role_files(&files)?;
    let manifest = RerankerManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        kind: MANIFEST_KIND.to_string(),
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        source_url: Some(preset.source_url()),
        downloaded_at_epoch: 0,
        roles,
        files,
    };
    write_manifest(install_dir, &manifest)
}
