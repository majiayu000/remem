use std::path::Path;

use anyhow::{bail, Context, Result};

use super::artifacts::{relative_path_string, resolve_relative_symlink};
use super::locks::open_or_create_model_lock;
use super::{
    collect_model_artifacts, ensure_sorted_unique_paths, read_verified_manifest_unlocked,
    verify_manifest_file, write_manifest, VerifiedLocalManifest,
};
use crate::retrieval::embedding::local_semantic::{
    checked_relative_path, LocalEmbeddingPreset, LocalModelFile, LocalModelManifest,
    FASTEMBED_RUNTIME, MANIFEST_FILE, MANIFEST_SCHEMA_VERSION, MODEL_DOWNLOAD_LOCK_FILE,
    MODEL_STATE_LOCK_FILE,
};

const LEGACY_MANIFEST_SCHEMA_VERSION: u32 = 1;

pub(super) fn upgrade_schema_v1_manifest(
    install_dir: &Path,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<VerifiedLocalManifest> {
    let install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize {}", install_dir.display()))?;
    let (download_lock_path, download_lock) =
        open_or_create_model_lock(&install_dir, MODEL_DOWNLOAD_LOCK_FILE)
            .context("open local model upgrade serialization lock")?;
    match fs2::FileExt::try_lock_exclusive(&download_lock) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
            bail!(
                "local model manifest upgrade deferred while download is active: {}",
                download_lock_path.display()
            );
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "lock local model manifest upgrade {}",
                    download_lock_path.display()
                )
            });
        }
    }
    let (state_lock_path, state_lock) =
        open_or_create_model_lock(&install_dir, MODEL_STATE_LOCK_FILE)
            .context("open local model state lock for manifest upgrade")?;
    fs2::FileExt::lock_exclusive(&state_lock).with_context(|| {
        format!(
            "lock local model state for manifest upgrade {}",
            state_lock_path.display()
        )
    })?;
    super::recover_pending_activation(&install_dir)
        .context("recover interrupted model activation before manifest upgrade")?;

    if let Ok(verified) = read_verified_manifest_unlocked(&install_dir, expected_preset) {
        return Ok(verified);
    }

    let path = install_dir.join(MANIFEST_FILE);
    let metadata =
        std::fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "legacy local embedding manifest is not a regular file: {}",
            path.display()
        );
    }
    let content = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let legacy: LocalModelManifest =
        serde_json::from_slice(&content).with_context(|| format!("parse {}", path.display()))?;
    let preset = verify_legacy_header(&legacy, expected_preset)?;
    for file in &legacy.files {
        verify_legacy_manifest_file(&install_dir, file)?;
    }

    let (files, symlinks) = collect_model_artifacts(&install_dir, preset)?;
    ensure_artifacts_were_bound_by_legacy_manifest(&legacy.files, &files)?;
    let upgraded = LocalModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        dimensions: preset.dimensions(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        source_url: Some(preset.source_url()),
        downloaded_at_epoch: legacy.downloaded_at_epoch,
        files,
        symlinks,
    };
    write_manifest(&install_dir, &upgraded)?;
    read_verified_manifest_unlocked(&install_dir, Some(preset))
        .context("verify upgraded schema-v2 local embedding manifest")
}

fn verify_legacy_manifest_file(install_dir: &Path, file: &LocalModelFile) -> Result<()> {
    let path = install_dir.join(checked_relative_path(&file.path)?);
    let metadata =
        std::fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    if metadata.file_type().is_file() {
        return verify_manifest_file(install_dir, file);
    }
    if !metadata.file_type().is_symlink() {
        bail!("legacy manifest path is not a file: {}", path.display());
    }

    let target = std::fs::read_link(&path)
        .with_context(|| format!("read legacy manifest symlink {}", path.display()))?;
    let resolved = resolve_relative_symlink(install_dir, &path, &target)?;
    let mut resolved_file = file.clone();
    resolved_file.path = relative_path_string(install_dir, &resolved)?;
    verify_manifest_file(install_dir, &resolved_file)
}

fn verify_legacy_header(
    manifest: &LocalModelManifest,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<LocalEmbeddingPreset> {
    if manifest.schema_version != LEGACY_MANIFEST_SCHEMA_VERSION {
        bail!(
            "legacy manifest upgrade requires schema {}, got {}",
            LEGACY_MANIFEST_SCHEMA_VERSION,
            manifest.schema_version
        );
    }
    if !manifest.symlinks.is_empty() {
        bail!("schema-v1 local embedding manifest unexpectedly declares symlinks");
    }
    let preset = LocalEmbeddingPreset::parse(&manifest.preset)?;
    if expected_preset.is_some_and(|expected| expected != preset) {
        bail!(
            "legacy manifest preset {} does not match expected {}",
            manifest.preset,
            expected_preset
                .map(LocalEmbeddingPreset::label)
                .unwrap_or("<unknown>")
        );
    }
    if manifest.model_id != preset.model_id() {
        bail!(
            "legacy manifest model_id {} does not match preset {}",
            manifest.model_id,
            preset.model_id()
        );
    }
    if manifest.upstream_model != preset.upstream_model() {
        bail!(
            "legacy manifest upstream_model {} does not match preset {} upstream {}",
            manifest.upstream_model,
            preset.label(),
            preset.upstream_model()
        );
    }
    if manifest.dimensions != preset.dimensions() {
        bail!(
            "legacy manifest dimensions {} do not match preset {} dimensions {}",
            manifest.dimensions,
            preset.label(),
            preset.dimensions()
        );
    }
    if manifest.runtime != FASTEMBED_RUNTIME {
        bail!(
            "unsupported legacy local embedding runtime {}",
            manifest.runtime
        );
    }
    if let Some(source_url) = manifest.source_url.as_deref() {
        let expected = preset.source_url();
        if source_url != expected {
            bail!(
                "legacy manifest source_url {} does not match preset {} source {}",
                source_url,
                preset.label(),
                expected
            );
        }
    }
    if manifest.files.is_empty() {
        bail!("legacy local embedding manifest has no verified files");
    }
    ensure_sorted_unique_paths(
        manifest.files.iter().map(|file| file.path.as_str()),
        "legacy file",
    )?;
    Ok(preset)
}

fn ensure_artifacts_were_bound_by_legacy_manifest(
    legacy_files: &[LocalModelFile],
    upgraded_files: &[LocalModelFile],
) -> Result<()> {
    for upgraded in upgraded_files {
        let Some(legacy) = legacy_files
            .iter()
            .find(|legacy| legacy.path == upgraded.path)
        else {
            bail!(
                "active local model artifact {} was not bound by schema-v1 manifest; re-download the local embedding model",
                upgraded.path
            );
        };
        if legacy.bytes != upgraded.bytes || legacy.sha256 != upgraded.sha256 {
            bail!(
                "active local model artifact {} differs from schema-v1 manifest; re-download the local embedding model",
                upgraded.path
            );
        }
    }
    Ok(())
}
