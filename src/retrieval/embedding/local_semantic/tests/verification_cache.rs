use anyhow::{Context, Result};

use super::super::*;

#[test]
#[cfg(feature = "local-onnx")]
fn cached_verification_rehashes_same_size_regular_file_corruption() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let model_root = std::env::temp_dir().join(format!(
        "remem-manifest-regular-corruption-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    install_test_model(&model_root)?;
    let config = EmbeddingConfig {
        model_dir: Some(model_root.display().to_string()),
        ..EmbeddingConfig::default()
    };
    installed_model_profile(&config)?;
    let runtime_file = test_model_runtime_file(&model_root, "config.json");
    let mut corrupted = std::fs::read(&runtime_file)?;
    let first = corrupted
        .first_mut()
        .context("test model config must not be empty")?;
    *first ^= 0xff;
    std::fs::write(&runtime_file, &corrupted)?;

    let error = installed_model_profile(&config).unwrap_err();

    assert!(error.to_string().contains("checksum mismatch"), "{error:#}");
    std::fs::remove_dir_all(&model_root)
        .with_context(|| format!("remove test model root {}", model_root.display()))?;
    Ok(())
}
