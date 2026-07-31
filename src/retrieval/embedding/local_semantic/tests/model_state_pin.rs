#![cfg(feature = "local-onnx")]

use std::time::Duration;

use anyhow::{Context, Result};

use super::super::*;
use crate::retrieval::embedding::EmbeddingProvider;

#[test]
fn configured_model_read_lock_blocks_activation_until_evaluation_finishes() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let model_root = super::test_download_root("evaluation-model-pin");
    install_test_model(&model_root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    let config = EmbeddingConfig {
        provider: EmbeddingProvider::Local,
        model_dir: Some(model_root.display().to_string()),
        ..EmbeddingConfig::default()
    };
    let (attempted_tx, attempted_rx) = std::sync::mpsc::sync_channel(1);
    let (acquired_tx, acquired_rx) = std::sync::mpsc::sync_channel(1);

    let writer = with_configured_model_read_lock(&config, || {
        let writer = std::thread::spawn(move || -> Result<()> {
            let (_, state_lock) =
                manifest::open_or_create_model_lock(&install_dir, MODEL_STATE_LOCK_FILE)?;
            attempted_tx.send(())?;
            fs2::FileExt::lock_exclusive(&state_lock)?;
            acquired_tx.send(())?;
            Ok(())
        });
        attempted_rx
            .recv_timeout(Duration::from_secs(1))
            .context("state writer did not attempt the model lock")?;
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "state writer acquired while the evaluation model pin was alive"
        );
        Ok(writer)
    })?;

    writer
        .join()
        .map_err(|_| anyhow::anyhow!("state writer panicked"))??;
    acquired_rx
        .recv_timeout(Duration::from_secs(1))
        .context("state writer stayed blocked after the evaluation model pin dropped")?;
    std::fs::remove_dir_all(&model_root)
        .with_context(|| format!("remove evaluation model pin root {}", model_root.display()))
}
