use std::path::{Path, PathBuf};

#[cfg(any(feature = "local-onnx", test))]
use anyhow::Context;
use anyhow::Result;

#[cfg(feature = "local-onnx")]
use super::windows_security;
use super::windows_security::DirectoryAnchor;
use super::{model_root, model_unavailable_error, EmbeddingConfig, LocalEmbeddingPreset};

pub(super) struct ManagedInstall {
    install_dir: PathBuf,
    _model_root: DirectoryAnchor,
    _install: DirectoryAnchor,
}

impl ManagedInstall {
    pub(super) fn install_dir(&self) -> &Path {
        &self.install_dir
    }
}

pub(super) fn checked_model_root(config: &EmbeddingConfig) -> Result<PathBuf> {
    let root = model_root(config);
    let default = dirs::home_dir().map(|home| home.join(".remem/models"));
    let non_default = config.model_dir.is_some() || default.as_ref() != Some(&root);
    #[cfg(test)]
    let registered = super::test_support::is_windows_secure_test_model_root(&root)?;
    #[cfg(not(test))]
    let registered = false;
    if non_default && !registered {
        return Err(policy_error(
            "embeddings.model_dir and non-default REMEM_DATA_DIR roots are not supported",
        ));
    }
    if !root.is_absolute() {
        return Err(policy_error(
            "the default per-user model root is unavailable",
        ));
    }
    Ok(root)
}

pub(super) fn open_managed_install(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
    optional: bool,
) -> Result<Option<ManagedInstall>> {
    let root = checked_model_root(config)?;
    if optional && path_is_missing(&root) {
        return Ok(None);
    }
    let root_anchor = DirectoryAnchor::open_owner_only(&root, false)
        .map_err(|_| policy_error("model root owner/DACL/identity validation failed"))?;
    let install_dir = root.join(preset.model_id());
    if optional && path_is_missing(&install_dir) {
        return Ok(None);
    }
    let install = DirectoryAnchor::open_owner_only(&install_dir, false)
        .map_err(|_| policy_error("model install owner/DACL/identity validation failed"))?;
    root_anchor
        .verify_path()
        .map_err(|_| policy_error("model root identity changed during validation"))?;
    install
        .verify_path()
        .map_err(|_| policy_error("model install identity changed during validation"))?;
    Ok(Some(ManagedInstall {
        install_dir,
        _model_root: root_anchor,
        _install: install,
    }))
}

#[cfg(feature = "local-onnx")]
pub(super) fn create_managed_install(
    config: &EmbeddingConfig,
    preset: LocalEmbeddingPreset,
) -> Result<ManagedInstall> {
    let root = checked_model_root(config)?;
    let parent = root
        .parent()
        .context("default model root must have a parent")
        .map_err(|_| policy_error("the default per-user data directory is unavailable"))?;
    std::fs::create_dir_all(parent)
        .map_err(|_| policy_error("per-user data directory creation failed"))?;
    let root_anchor = windows_security::open_or_create_owner_only_directory(&root)
        .map_err(|_| policy_error("model root secure creation or validation failed"))?;
    let install_dir = root.join(preset.model_id());
    let install = windows_security::open_or_create_owner_only_directory(&install_dir)
        .map_err(|_| policy_error("model install secure creation or validation failed"))?;
    root_anchor
        .verify_path()
        .map_err(|_| policy_error("model root identity changed during creation"))?;
    install
        .verify_path()
        .map_err(|_| policy_error("model install identity changed during creation"))?;
    Ok(ManagedInstall {
        install_dir,
        _model_root: root_anchor,
        _install: install,
    })
}

fn path_is_missing(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

fn policy_error(reason: &str) -> anyhow::Error {
    model_unavailable_error(format!(
        "Windows local embedding security policy rejected the model root: {reason}. Only the default per-user model root is supported. Existing caches with broad or inherited ACLs are not repaired automatically; delete the local model cache and run `remem embedding download` to recreate it securely"
    ))
}

#[cfg(feature = "local-onnx")]
pub(super) fn missing_install_error() -> anyhow::Error {
    policy_error("the owner-only model install does not exist")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::embedding::EmbeddingProvider;

    #[test]
    fn default_per_user_root_remains_supported() -> Result<()> {
        let _guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let _restore = EnvRestore::capture("REMEM_DATA_DIR");
        unsafe { std::env::remove_var("REMEM_DATA_DIR") };
        let expected = dirs::home_dir()
            .context("Windows test requires a per-user home directory")?
            .join(".remem/models");

        assert_eq!(checked_model_root(&EmbeddingConfig::default())?, expected);
        Ok(())
    }

    #[test]
    fn worker_exported_default_data_dir_remains_supported() -> Result<()> {
        let _guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let _restore = EnvRestore::capture("REMEM_DATA_DIR");
        let default_data_dir = dirs::home_dir()
            .context("Windows test requires a per-user home directory")?
            .join(".remem");
        unsafe { std::env::set_var("REMEM_DATA_DIR", &default_data_dir) };

        assert_eq!(
            checked_model_root(&EmbeddingConfig::default())?,
            default_data_dir.join("models")
        );
        Ok(())
    }

    #[test]
    fn explicit_bge_custom_root_is_typed_and_redacted() {
        let secret = std::env::temp_dir().join("sensitive-custom-model-root");
        let config = EmbeddingConfig {
            provider: EmbeddingProvider::Local,
            model: "bge-m3".to_string(),
            model_dir: Some(secret.display().to_string()),
            ..EmbeddingConfig::default()
        };

        let error = checked_model_root(&config).unwrap_err();
        let message = error.to_string();

        assert!(super::super::is_model_unavailable_error(&error));
        assert!(message.contains("default per-user model root"));
        assert!(message.contains("delete the local model cache"));
        assert!(!message.contains("sensitive-custom-model-root"));
    }

    #[test]
    fn remem_data_dir_override_is_typed_and_redacted() {
        let _guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let _restore = EnvRestore::capture("REMEM_DATA_DIR");
        unsafe {
            std::env::set_var(
                "REMEM_DATA_DIR",
                std::env::temp_dir().join("sensitive-shared-data-root"),
            )
        };

        let error = checked_model_root(&EmbeddingConfig::default()).unwrap_err();
        let message = error.to_string();

        assert!(super::super::is_model_unavailable_error(&error));
        assert!(message.contains("REMEM_DATA_DIR"));
        assert!(!message.contains("sensitive-shared-data-root"));
    }

    #[test]
    #[cfg(not(feature = "local-onnx"))]
    fn no_feature_entrypoints_apply_windows_root_policy_first() {
        let _guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let _data_restore = EnvRestore::capture("REMEM_DATA_DIR");
        let _model_restore = EnvRestore::capture("REMEM_EMBEDDINGS_MODEL_DIR");
        let _config_restore = EnvRestore::capture("REMEM_CONFIG");
        unsafe {
            std::env::remove_var("REMEM_DATA_DIR");
            std::env::set_var(
                "REMEM_EMBEDDINGS_MODEL_DIR",
                std::env::temp_dir().join("sensitive-no-feature-root"),
            );
            std::env::set_var(
                "REMEM_CONFIG",
                std::env::temp_dir().join("remem-missing-no-feature-config.toml"),
            );
        }
        let config = EmbeddingConfig {
            provider: EmbeddingProvider::Local,
            model: "bge-m3".to_string(),
            model_dir: Some("sensitive-no-feature-root".to_string()),
            ..EmbeddingConfig::default()
        };
        let profile_error = super::super::installed_model_profile(&config).unwrap_err();
        let embed_error = super::super::embed_text(
            "text",
            &config,
            super::super::LocalEmbeddingInputKind::Generic,
        )
        .unwrap_err();
        let download_error = super::super::download_model(Some("bge-m3")).unwrap_err();

        for error in [&profile_error, &embed_error, &download_error] {
            assert!(super::super::is_model_unavailable_error(error));
            assert!(error.to_string().contains("default per-user model root"));
            assert!(!error.to_string().contains("sensitive-no-feature-root"));
        }
    }

    #[test]
    #[cfg(feature = "local-onnx")]
    fn registered_secure_fixture_is_the_only_test_override() -> Result<()> {
        let root = test_root("registered");
        super::super::test_support::install_test_model(&root)?;
        let config = EmbeddingConfig {
            provider: EmbeddingProvider::Local,
            model: "multilingual-e5-small".to_string(),
            model_dir: Some(root.display().to_string()),
            ..EmbeddingConfig::default()
        };

        let install = open_managed_install(&config, LocalEmbeddingPreset::default(), false)?
            .context("registered secure fixture should open")?;

        drop(install);
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    struct EnvRestore {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvRestore {
        fn capture(key: &'static str) -> Self {
            Self {
                key,
                previous: std::env::var_os(key),
            }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => unsafe { std::env::set_var(self.key, value) },
                None => unsafe { std::env::remove_var(self.key) },
            }
        }
    }

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remem-windows-model-root-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
