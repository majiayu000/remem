#[cfg(feature = "local-onnx")]
use std::collections::HashMap;
#[cfg(any(feature = "local-onnx", windows))]
use std::collections::HashSet;
use std::path::Path;
#[cfg(any(feature = "local-onnx", windows))]
use std::sync::{Mutex, OnceLock};

#[cfg(any(feature = "local-onnx", windows))]
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::{
    collect_model_artifacts, write_manifest, LocalEmbeddingPreset, LocalModelManifest,
    FASTEMBED_RUNTIME, MANIFEST_FILE, MANIFEST_SCHEMA_VERSION, MODEL_DOWNLOAD_LOCK_FILE,
};

#[cfg(feature = "local-onnx")]
static TEST_RUNTIME_READINESS_OVERRIDES: OnceLock<
    Mutex<HashMap<PathBuf, TestRuntimeReadinessOverride>>,
> = OnceLock::new();
#[cfg(feature = "local-onnx")]
static TEST_AUTO_TRUSTED_INSTALLS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
#[cfg(feature = "local-onnx")]
static TEST_NEXT_EMBED_FAILURES: OnceLock<Mutex<HashMap<PathBuf, TestEmbedFailure>>> =
    OnceLock::new();
#[cfg(windows)]
static TEST_WINDOWS_SECURE_MODEL_ROOTS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

#[cfg(windows)]
pub(super) fn is_windows_secure_test_model_root(root: &Path) -> Result<bool> {
    Ok(TEST_WINDOWS_SECURE_MODEL_ROOTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| anyhow::anyhow!("Windows secure test model registry lock poisoned"))?
        .contains(root))
}

#[cfg(windows)]
fn register_windows_secure_test_model_root(root: &Path) -> Result<()> {
    TEST_WINDOWS_SECURE_MODEL_ROOTS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .map_err(|_| anyhow::anyhow!("Windows secure test model registry lock poisoned"))?
        .insert(root.to_path_buf());
    Ok(())
}

#[cfg(feature = "local-onnx")]
#[derive(Clone)]
pub(super) enum TestRuntimeReadinessOverride {
    Ready,
    Fail(String),
}

#[cfg(feature = "local-onnx")]
#[derive(Clone)]
pub(super) enum TestEmbedFailure {
    ModelUnavailable(String),
    Generic(String),
}

#[cfg(feature = "local-onnx")]
pub(crate) struct TestRuntimeReadinessFailure {
    install_dir: PathBuf,
    previous: Option<TestRuntimeReadinessOverride>,
}

#[cfg(feature = "local-onnx")]
impl Drop for TestRuntimeReadinessFailure {
    fn drop(&mut self) {
        let mut overrides = test_runtime_readiness_overrides()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match self.previous.take() {
            Some(previous) => {
                overrides.insert(self.install_dir.clone(), previous);
            }
            None => {
                overrides.remove(&self.install_dir);
            }
        }
    }
}

#[cfg(feature = "local-onnx")]
pub(crate) struct TestNextEmbedFailure {
    install_dir: PathBuf,
    previous: Option<TestEmbedFailure>,
}

#[cfg(feature = "local-onnx")]
impl Drop for TestNextEmbedFailure {
    fn drop(&mut self) {
        let mut failures = test_next_embed_failures()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match self.previous.take() {
            Some(previous) => {
                failures.insert(self.install_dir.clone(), previous);
            }
            None => {
                failures.remove(&self.install_dir);
            }
        }
    }
}

pub(crate) fn install_test_model(model_root: &Path) -> Result<()> {
    install_test_model_with_schema(
        model_root,
        LocalEmbeddingPreset::default(),
        MANIFEST_SCHEMA_VERSION,
    )?;
    #[cfg(feature = "local-onnx")]
    {
        register_test_runtime_ready(model_root, LocalEmbeddingPreset::default())?;
        register_test_auto_trusted(model_root, LocalEmbeddingPreset::default())?;
    }
    Ok(())
}

#[cfg(feature = "local-onnx")]
pub(crate) fn install_test_model_v1(model_root: &Path) -> Result<()> {
    install_test_model_with_schema(model_root, LocalEmbeddingPreset::default(), 1)?;
    register_test_runtime_ready(model_root, LocalEmbeddingPreset::default())?;
    register_test_auto_trusted(model_root, LocalEmbeddingPreset::default())
}

#[cfg(feature = "local-onnx")]
pub(crate) fn install_untrusted_test_model(model_root: &Path) -> Result<()> {
    install_test_model_with_schema(
        model_root,
        LocalEmbeddingPreset::default(),
        MANIFEST_SCHEMA_VERSION,
    )
}

#[cfg(feature = "local-onnx")]
pub(super) fn install_test_model_for_preset(
    model_root: &Path,
    preset: LocalEmbeddingPreset,
) -> Result<()> {
    install_test_model_with_schema(model_root, preset, MANIFEST_SCHEMA_VERSION)
}

#[cfg(feature = "local-onnx")]
pub(crate) fn fail_test_model_runtime_readiness(
    model_root: &Path,
    reason: impl Into<String>,
) -> Result<TestRuntimeReadinessFailure> {
    let install_dir =
        std::fs::canonicalize(model_root.join(LocalEmbeddingPreset::default().model_id()))
            .context("canonicalize test local embedding install dir")?;
    let mut overrides = test_runtime_readiness_overrides()
        .lock()
        .map_err(|_| anyhow::anyhow!("test runtime readiness failure lock poisoned"))?;
    let previous = overrides.insert(
        install_dir.clone(),
        TestRuntimeReadinessOverride::Fail(reason.into()),
    );
    Ok(TestRuntimeReadinessFailure {
        install_dir,
        previous,
    })
}

#[cfg(feature = "local-onnx")]
pub(crate) fn fail_next_test_model_embed_unavailable(
    model_root: &Path,
    reason: impl Into<String>,
) -> Result<TestNextEmbedFailure> {
    register_next_embed_failure(
        model_root,
        TestEmbedFailure::ModelUnavailable(reason.into()),
    )
}

#[cfg(feature = "local-onnx")]
pub(crate) fn fail_next_test_model_embed_generic(
    model_root: &Path,
    reason: impl Into<String>,
) -> Result<TestNextEmbedFailure> {
    register_next_embed_failure(model_root, TestEmbedFailure::Generic(reason.into()))
}

#[cfg(feature = "local-onnx")]
fn register_next_embed_failure(
    model_root: &Path,
    failure: TestEmbedFailure,
) -> Result<TestNextEmbedFailure> {
    let install_dir =
        std::fs::canonicalize(model_root.join(LocalEmbeddingPreset::default().model_id()))
            .context("canonicalize test local embedding install dir")?;
    let previous = test_next_embed_failures()
        .lock()
        .map_err(|_| anyhow::anyhow!("test next embedding failure lock poisoned"))?
        .insert(install_dir.clone(), failure);
    Ok(TestNextEmbedFailure {
        install_dir,
        previous,
    })
}

#[cfg(feature = "local-onnx")]
pub(super) fn take_next_embed_failure(install_dir: &Path) -> Result<Option<TestEmbedFailure>> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize test install dir {}", install_dir.display()))?;
    Ok(test_next_embed_failures()
        .lock()
        .map_err(|_| anyhow::anyhow!("test next embedding failure lock poisoned"))?
        .remove(&canonical_install_dir))
}

#[cfg(feature = "local-onnx")]
pub(super) fn runtime_readiness_override(
    install_dir: &Path,
) -> Result<Option<TestRuntimeReadinessOverride>> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize test install dir {}", install_dir.display()))?;
    Ok(test_runtime_readiness_overrides()
        .lock()
        .map_err(|_| anyhow::anyhow!("test runtime readiness failure lock poisoned"))?
        .get(&canonical_install_dir)
        .cloned())
}

#[cfg(feature = "local-onnx")]
pub(super) fn is_test_auto_artifact_trusted(install_dir: &Path) -> Result<bool> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize test install dir {}", install_dir.display()))?;
    Ok(test_auto_trusted_installs()
        .lock()
        .map_err(|_| anyhow::anyhow!("test auto artifact trust lock poisoned"))?
        .contains(&canonical_install_dir))
}

#[cfg(feature = "local-onnx")]
fn register_test_runtime_ready(model_root: &Path, preset: LocalEmbeddingPreset) -> Result<()> {
    let install_dir = std::fs::canonicalize(model_root.join(preset.model_id()))
        .context("canonicalize test local embedding install dir")?;
    let mut overrides = test_runtime_readiness_overrides()
        .lock()
        .map_err(|_| anyhow::anyhow!("test runtime readiness override lock poisoned"))?;
    overrides.insert(install_dir, TestRuntimeReadinessOverride::Ready);
    Ok(())
}

#[cfg(feature = "local-onnx")]
fn register_test_auto_trusted(model_root: &Path, preset: LocalEmbeddingPreset) -> Result<()> {
    let install_dir = std::fs::canonicalize(model_root.join(preset.model_id()))
        .context("canonicalize trusted test local embedding install dir")?;
    test_auto_trusted_installs()
        .lock()
        .map_err(|_| anyhow::anyhow!("test auto artifact trust lock poisoned"))?
        .insert(install_dir);
    Ok(())
}

#[cfg(feature = "local-onnx")]
fn test_runtime_readiness_overrides(
) -> &'static Mutex<HashMap<PathBuf, TestRuntimeReadinessOverride>> {
    TEST_RUNTIME_READINESS_OVERRIDES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(feature = "local-onnx")]
fn test_auto_trusted_installs() -> &'static Mutex<HashSet<PathBuf>> {
    TEST_AUTO_TRUSTED_INSTALLS.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(feature = "local-onnx")]
fn test_next_embed_failures() -> &'static Mutex<HashMap<PathBuf, TestEmbedFailure>> {
    TEST_NEXT_EMBED_FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn install_test_model_with_schema(
    model_root: &Path,
    preset: LocalEmbeddingPreset,
    schema_version: u32,
) -> Result<()> {
    let install_dir = model_root.join(preset.model_id());
    #[cfg(windows)]
    let (_model_root_anchor, _install_anchor) = {
        let root = super::windows_security::open_or_create_owner_only_directory(model_root)
            .with_context(|| format!("secure test model root {}", model_root.display()))?;
        let install = super::windows_security::open_or_create_owner_only_directory(&install_dir)
            .with_context(|| format!("secure test model dir {}", install_dir.display()))?;
        register_windows_secure_test_model_root(model_root)?;
        (root, install)
    };
    #[cfg(not(windows))]
    std::fs::create_dir_all(&install_dir)
        .with_context(|| format!("create test model dir {}", install_dir.display()))?;
    #[cfg(windows)]
    drop(
        super::windows_security::create_owner_only_lock_file(
            &install_dir.join(MODEL_DOWNLOAD_LOCK_FILE),
        )
        .with_context(|| format!("create secure test model lock {}", install_dir.display()))?,
    );
    #[cfg(not(windows))]
    std::fs::write(install_dir.join(MODEL_DOWNLOAD_LOCK_FILE), b"")
        .with_context(|| format!("write test model lock {}", install_dir.display()))?;
    let revision = test_revision();
    let repo_dir = preset.cache_repo_dir();
    let ref_path = install_dir.join(&repo_dir).join("refs/main");
    std::fs::create_dir_all(
        ref_path
            .parent()
            .context("test model revision ref should have a parent")?,
    )?;
    std::fs::write(&ref_path, &revision)
        .with_context(|| format!("write test model revision {}", ref_path.display()))?;
    for runtime_file in preset.required_runtime_files() {
        let path = install_dir
            .join(&repo_dir)
            .join("snapshots")
            .join(&revision)
            .join(runtime_file);
        std::fs::create_dir_all(
            path.parent()
                .context("test runtime file should have a parent")?,
        )?;
        std::fs::write(&path, format!("deterministic-test-model:{runtime_file}"))
            .with_context(|| format!("write test model fixture {}", path.display()))?;
    }
    let (files, symlinks) = collect_model_artifacts(&install_dir, preset)?;
    let manifest = LocalModelManifest {
        schema_version,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        dimensions: preset.dimensions(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        source_url: Some(preset.source_url()),
        downloaded_at_epoch: 0,
        files,
        symlinks: if schema_version == 1 {
            Vec::new()
        } else {
            symlinks
        },
    };
    if schema_version == 1 {
        let mut value = serde_json::to_value(&manifest)?;
        value
            .as_object_mut()
            .context("test model manifest should be an object")?
            .remove("symlinks");
        std::fs::write(
            install_dir.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(&value)?,
        )?;
        std::fs::remove_file(install_dir.join(MODEL_DOWNLOAD_LOCK_FILE))?;
        return Ok(());
    }
    write_manifest(&install_dir, &manifest)
}

#[cfg(feature = "local-onnx")]
pub(super) fn create_test_owner_only_install(model_root: &Path, install_dir: &Path) -> Result<()> {
    if install_dir.parent() != Some(model_root) {
        anyhow::bail!("test model install must be a direct child of its model root");
    }
    #[cfg(windows)]
    {
        let root = super::windows_security::open_or_create_owner_only_directory(model_root)?;
        let install = super::windows_security::open_or_create_owner_only_directory(install_dir)?;
        register_windows_secure_test_model_root(model_root)?;
        drop(install);
        drop(root);
    }
    #[cfg(not(windows))]
    std::fs::create_dir_all(install_dir)?;
    Ok(())
}

#[cfg(feature = "local-onnx")]
pub(crate) fn test_model_runtime_file(model_root: &Path, runtime_file: &str) -> PathBuf {
    let preset = LocalEmbeddingPreset::default();
    model_root
        .join(preset.model_id())
        .join(preset.cache_repo_dir())
        .join("snapshots")
        .join(test_revision())
        .join(runtime_file)
}

fn test_revision() -> String {
    "a".repeat(40)
}

#[cfg(windows)]
mod windows_security_tests {
    use std::io;

    use super::*;
    use crate::retrieval::embedding::local_semantic::windows_security::{self, DirectoryAnchor};

    #[test]
    fn directory_anchor_blocks_replacement_until_drop() -> io::Result<()> {
        let root = test_root("directory-anchor");
        std::fs::create_dir(&root)?;
        let path = root.join("secure");
        let anchor = windows_security::create_owner_only_directory(&path, false)?;

        assert!(std::fs::remove_dir(&path).is_err());
        anchor.verify_path()?;
        drop(anchor);
        std::fs::remove_dir(&path)?;
        std::fs::remove_dir(&root)
    }

    #[test]
    fn renameable_anchor_preserves_full_published_identity() -> io::Result<()> {
        let root = test_root("renameable-anchor");
        std::fs::create_dir(&root)?;
        let staging_path = root.join("staging");
        let published_path = root.join("published");
        let staging = windows_security::create_owner_only_directory(&staging_path, true)?;
        let expected = staging.identity();

        std::fs::rename(&staging_path, &published_path)?;
        let published = DirectoryAnchor::open_owner_only(&published_path, false)?;

        assert_eq!(published.identity(), expected);
        drop(staging);
        drop(published);
        std::fs::remove_dir(&published_path)?;
        std::fs::remove_dir(&root)
    }

    #[test]
    fn ordinary_inherited_dacl_is_rejected_without_repair() -> io::Result<()> {
        let root = test_root("inherited-dacl");
        std::fs::create_dir(&root)?;
        let inherited_dir = root.join("ordinary");
        std::fs::create_dir(&inherited_dir)?;
        let inherited_lock = root.join("ordinary.lock");
        std::fs::write(&inherited_lock, b"")?;

        let directory_error = DirectoryAnchor::open_owner_only(&inherited_dir, false).unwrap_err();
        let lock_error = windows_security::open_owner_only_lock_file(&inherited_lock).unwrap_err();

        assert!(directory_error.to_string().contains("DACL"));
        assert!(lock_error.to_string().contains("DACL"));
        assert!(inherited_dir.exists());
        assert!(inherited_lock.exists());
        std::fs::remove_file(&inherited_lock)?;
        std::fs::remove_dir(&inherited_dir)?;
        std::fs::remove_dir(&root)
    }

    #[test]
    fn lock_collision_opens_existing_full_identity() -> io::Result<()> {
        let root = test_root("lock-collision");
        std::fs::create_dir(&root)?;
        let path = root.join("state.lock");
        let first = windows_security::create_owner_only_lock_file(&path)?;
        let expected = windows_security::lock_file_identity(&first)?;

        let existing = windows_security::open_or_create_owner_only_lock_file(&path)?;

        assert_eq!(windows_security::lock_file_identity(&existing)?, expected);
        windows_security::validate_lock_file(&path, &first)?;
        windows_security::validate_lock_file(&path, &existing)?;
        assert!(std::fs::remove_file(&path).is_err());
        drop(existing);
        drop(first);
        std::fs::remove_file(&path)?;
        std::fs::remove_dir(&root)
    }

    #[test]
    fn malicious_access_ace_lengths_are_rejected_before_sid_reads() {
        for size in [4usize, 8, 12, 15] {
            let ace = access_ace_bytes(size, size);
            assert!(
                windows_security::validate_access_ace_layout_for_test(&ace).is_err(),
                "AceSize {size} must be rejected"
            );
        }

        let mut sid_overrun = access_ace_bytes(16, 16);
        sid_overrun[9] = 1;
        assert!(windows_security::validate_access_ace_layout_for_test(&sid_overrun).is_err());

        let outside_acl = access_ace_bytes(16, 20);
        assert!(windows_security::validate_access_ace_layout_for_test(&outside_acl).is_err());
    }

    #[test]
    #[cfg(feature = "local-onnx")]
    fn model_fixture_reopens_its_secure_lock() -> Result<()> {
        let root = test_root("fixture-lock");
        install_test_model(&root)?;
        let install_dir = root.join(LocalEmbeddingPreset::default().model_id());

        let (_, lock) = super::super::manifest::open_or_create_model_lock(
            &install_dir,
            MODEL_DOWNLOAD_LOCK_FILE,
        )?;

        drop(lock);
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

    fn access_ace_bytes(buffer_size: usize, declared_size: usize) -> Vec<u8> {
        let mut bytes = vec![0; buffer_size];
        if buffer_size >= 4 {
            bytes[2..4].copy_from_slice(&(declared_size as u16).to_le_bytes());
        }
        bytes
    }

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "remem-windows-security-{label}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }
}
