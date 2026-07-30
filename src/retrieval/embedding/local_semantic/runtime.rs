use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::manifest::verified_runtime_file;
use super::{
    LocalEmbeddingInputKind, LocalEmbeddingPreset, LocalModelFile, LocalModelManifest,
    TOKENIZER_RUNTIME_FILES,
};

mod file_backed_text;
mod private_cache;

use file_backed_text::FileBackedTextEmbedding;
use private_cache::PrivateRuntimeCache;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LocalModelCacheKey {
    preset: LocalEmbeddingPreset,
    install_dir: PathBuf,
}

struct CachedLocalModel<M> {
    artifact_sha256: String,
    model: M,
}

const LOCAL_MODEL_CACHE_CAPACITY: usize = 2;

struct ProcessModelCache<M> {
    capacity: usize,
    entries: HashMap<LocalModelCacheKey, CachedLocalModel<M>>,
}

impl<M> ProcessModelCache<M> {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: HashMap::new(),
        }
    }

    fn with_model<T>(
        &mut self,
        key: LocalModelCacheKey,
        artifact_sha256: &str,
        load: impl FnOnce() -> Result<M>,
        operation: impl FnOnce(&mut M) -> Result<T>,
    ) -> Result<T> {
        let needs_reload = self
            .entries
            .get(&key)
            .is_none_or(|cached| cached.artifact_sha256 != artifact_sha256);
        if needs_reload {
            self.entries.remove(&key);
            if self.entries.len() >= self.capacity {
                let evicted = self.entries.keys().next().cloned();
                if let Some(evicted) = evicted {
                    self.entries.remove(&evicted);
                }
            }
            let model = load()?;
            self.entries.insert(
                key.clone(),
                CachedLocalModel {
                    artifact_sha256: artifact_sha256.to_string(),
                    model,
                },
            );
        }
        let cached = self
            .entries
            .get_mut(&key)
            .context("local embedding model cache entry missing after initialization")?;
        operation(&mut cached.model)
    }
}

static LOCAL_MODEL_CACHE: OnceLock<Mutex<ProcessModelCache<LoadedLocalModel>>> = OnceLock::new();

enum LoadedLocalModel {
    FastEmbed {
        model: Box<fastembed::TextEmbedding>,
    },
    FileBacked {
        model: Box<FileBackedTextEmbedding>,
        _private_runtime_cache: PrivateRuntimeCache,
    },
    #[cfg(test)]
    TestReady,
}

impl LoadedLocalModel {
    fn embed(&mut self, input: &str) -> Result<Vec<f32>> {
        match self {
            Self::FastEmbed { model } => {
                let mut embeddings = model.embed([input], Some(1))?;
                let first = embeddings
                    .pop()
                    .context("local embedding model did not return an embedding")?;
                if !embeddings.is_empty() {
                    bail!("local embedding model returned multiple embeddings for single input");
                }
                Ok(first)
            }
            Self::FileBacked { model, .. } => model.embed(input),
            #[cfg(test)]
            Self::TestReady => bail!("synthetic local embedding fixture cannot run inference"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeLoadStrategy {
    VerifiedBytes,
    VerifiedFileBackedCache,
}

fn runtime_load_strategy(preset: LocalEmbeddingPreset) -> RuntimeLoadStrategy {
    match preset {
        LocalEmbeddingPreset::MultilingualE5Small => RuntimeLoadStrategy::VerifiedBytes,
        LocalEmbeddingPreset::BgeM3 => RuntimeLoadStrategy::VerifiedFileBackedCache,
    }
}

pub(super) fn embed_with_verified_model(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
    text: &str,
    kind: LocalEmbeddingInputKind,
) -> Result<Vec<f32>> {
    let input = preset.prefix_input(text, kind);
    with_verified_model(preset, install_dir, manifest, artifact_sha256, |model| {
        embed_with_loaded_model(model, preset, &input)
    })
}

pub(super) fn probe_verified_model(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
    text: &str,
    kind: LocalEmbeddingInputKind,
) -> Result<Vec<f32>> {
    let input = preset.prefix_input(text, kind);
    let mut model = load_verified_model(
        preset,
        &canonicalize_verified_install_dir(install_dir)?,
        manifest,
        artifact_sha256,
    )?;
    embed_with_loaded_model(&mut model, preset, &input)
}

fn embed_with_loaded_model(
    model: &mut LoadedLocalModel,
    preset: LocalEmbeddingPreset,
    input: &str,
) -> Result<Vec<f32>> {
    model
        .embed(input)
        .with_context(|| format!("embed text with local model {}", preset.label()))
}

pub(super) fn ensure_verified_model_ready(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
) -> Result<()> {
    with_verified_model(preset, install_dir, manifest, artifact_sha256, |_| Ok(()))
}

fn with_verified_model<T>(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
    operation: impl FnOnce(&mut LoadedLocalModel) -> Result<T>,
) -> Result<T> {
    let install_dir = canonicalize_verified_install_dir(install_dir)?;
    let key = LocalModelCacheKey {
        preset,
        install_dir: install_dir.clone(),
    };
    let mut cache = LOCAL_MODEL_CACHE
        .get_or_init(|| Mutex::new(ProcessModelCache::new(LOCAL_MODEL_CACHE_CAPACITY)))
        .lock()
        .map_err(|_| anyhow::anyhow!("local embedding model cache lock poisoned"))?;
    cache.with_model(
        key,
        artifact_sha256,
        || load_verified_model(preset, &install_dir, manifest, artifact_sha256),
        operation,
    )
}

fn load_verified_model(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
) -> Result<LoadedLocalModel> {
    let result = (|| {
        #[cfg(test)]
        if let Some(runtime_override) =
            super::test_support::runtime_readiness_override(install_dir)?
        {
            return match runtime_override {
                super::test_support::TestRuntimeReadinessOverride::Ready => {
                    Ok(LoadedLocalModel::TestReady)
                }
                super::test_support::TestRuntimeReadinessOverride::Fail(reason) => {
                    Err(anyhow::anyhow!(reason)
                        .context("initialize verified local embedding runtime"))
                }
            };
        }
        match runtime_load_strategy(preset) {
            RuntimeLoadStrategy::VerifiedBytes => Ok(LoadedLocalModel::FastEmbed {
                model: Box::new(load_verified_bytes_model(preset, install_dir, manifest)?),
            }),
            RuntimeLoadStrategy::VerifiedFileBackedCache => {
                load_verified_file_backed_model(preset, install_dir, manifest, artifact_sha256)
            }
        }
    })();
    result.map_err(model_load_unavailable_error)
}

fn canonicalize_verified_install_dir(install_dir: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(install_dir).map_err(|error| {
        super::model_unavailable_error(format!(
            "canonicalize verified local model dir {}: {error}",
            install_dir.display()
        ))
    })
}

fn model_load_unavailable_error(error: anyhow::Error) -> anyhow::Error {
    if super::is_model_unavailable_error(&error) {
        error
    } else {
        super::model_unavailable_error(format!(
            "verified local embedding model became unavailable during runtime initialization: {error:#}"
        ))
    }
}

fn load_verified_bytes_model(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
) -> Result<fastembed::TextEmbedding> {
    let onnx_file =
        read_verified_runtime_bytes(install_dir, manifest, preset, preset.model_file())?;
    let tokenizer_files = fastembed::TokenizerFiles {
        tokenizer_file: read_verified_runtime_bytes(
            install_dir,
            manifest,
            preset,
            TOKENIZER_RUNTIME_FILES[0],
        )?,
        config_file: read_verified_runtime_bytes(
            install_dir,
            manifest,
            preset,
            TOKENIZER_RUNTIME_FILES[1],
        )?,
        special_tokens_map_file: read_verified_runtime_bytes(
            install_dir,
            manifest,
            preset,
            TOKENIZER_RUNTIME_FILES[2],
        )?,
        tokenizer_config_file: read_verified_runtime_bytes(
            install_dir,
            manifest,
            preset,
            TOKENIZER_RUNTIME_FILES[3],
        )?,
    };
    let fastembed_model = preset.fastembed_model();
    let model_info = fastembed::TextEmbedding::get_model_info(&fastembed_model)?;
    let mut user_model = fastembed::UserDefinedEmbeddingModel::new(onnx_file, tokenizer_files);
    user_model.pooling = fastembed::TextEmbedding::get_default_pooling_method(&fastembed_model);
    user_model.quantization = fastembed::TextEmbedding::get_quantization_mode(&fastembed_model);
    user_model.output_key = model_info.output_key.clone();
    for additional_file in preset.additional_model_files() {
        let bytes = read_verified_runtime_bytes(install_dir, manifest, preset, additional_file)?;
        let file_name = Path::new(additional_file)
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("invalid additional model file name {additional_file}"))?;
        user_model = user_model.with_external_initializer(file_name.to_string(), bytes);
    }
    let options: fastembed::InitOptionsUserDefined =
        fastembed::TextInitOptions::new(fastembed_model).into();
    fastembed::TextEmbedding::try_new_from_user_defined(user_model, options)
        .with_context(|| format!("initialize verified local model {}", preset.label()))
}

fn load_verified_file_backed_model(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
) -> Result<LoadedLocalModel> {
    let (model, private_runtime_cache) = load_verified_file_backed_model_with(
        preset,
        install_dir,
        manifest,
        artifact_sha256,
        |private_root| {
            FileBackedTextEmbedding::load(preset, artifact_sha256, private_root).with_context(
                || format!("initialize verified file-backed model {}", preset.label()),
            )
        },
    )?;
    Ok(LoadedLocalModel::FileBacked {
        model: Box::new(model),
        _private_runtime_cache: private_runtime_cache,
    })
}

fn load_verified_file_backed_model_with<M>(
    preset: LocalEmbeddingPreset,
    install_dir: &Path,
    manifest: &LocalModelManifest,
    artifact_sha256: &str,
    loader: impl FnOnce(&Path) -> Result<M>,
) -> Result<(M, PrivateRuntimeCache)> {
    let private_runtime_cache =
        PrivateRuntimeCache::materialize(install_dir, manifest, preset, artifact_sha256)?;
    private_runtime_cache
        .verify()
        .context("verify private runtime cache before model initialization")?;
    let model = loader(private_runtime_cache.root())?;
    private_runtime_cache
        .verify()
        .context("verify private runtime cache after model initialization")?;
    Ok((model, private_runtime_cache))
}

fn read_verified_runtime_bytes(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
    runtime_file: &str,
) -> Result<Vec<u8>> {
    let (expected, path) = verified_runtime_file(install_dir, manifest, preset, runtime_file)?;
    read_and_verify_file(&path, expected)
}

fn read_and_verify_file(path: &Path, expected: &LocalModelFile) -> Result<Vec<u8>> {
    let capacity = usize::try_from(expected.bytes)
        .with_context(|| format!("model artifact is too large to load: {}", path.display()))?;
    let mut file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    if bytes.len() as u64 != expected.bytes {
        bail!(
            "verified model artifact {} size changed while loading: expected {}, got {}",
            path.display(),
            expected.bytes,
            bytes.len()
        );
    }
    let actual = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual != expected.sha256 {
        bail!(
            "verified model artifact {} checksum changed while loading: expected {}, got {}",
            path.display(),
            expected.sha256,
            actual
        );
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "runtime/tests.rs"]
mod tests;
