use anyhow::Result;
use std::path::PathBuf;

use super::{EmbeddingConfig, LocalEmbeddingPreset};

pub(in crate::retrieval::embedding) fn model_root(config: &EmbeddingConfig) -> Result<PathBuf> {
    match config.model_dir.as_ref() {
        Some(path) => Ok(PathBuf::from(path)),
        None => Ok(crate::db::try_data_dir()?.join("models")),
    }
}

#[cfg(not(windows))]
pub(super) fn install_dir_for_preset(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
) -> Result<PathBuf> {
    Ok(model_root(config)?.join(preset.model_id()))
}
