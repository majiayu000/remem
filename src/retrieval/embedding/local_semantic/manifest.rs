use std::collections::HashSet;
use std::path::Path;
#[cfg(feature = "local-onnx")]
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::{
    checked_relative_path, sha256_file, LocalEmbeddingPreset, LocalModelFile, LocalModelManifest,
    LocalModelSymlink, FASTEMBED_RUNTIME, MANIFEST_FILE, MANIFEST_SCHEMA_VERSION,
};

mod artifacts;
mod cache;
mod content_digest;
mod legacy;
mod locks;
mod publish;
mod transaction;

pub(super) use artifacts::canonical_regular_path;
#[cfg(feature = "local-onnx")]
use artifacts::verify_runtime_layout_at_revision;
use artifacts::{relative_path_string, resolve_relative_symlink, verify_runtime_layout};
use cache::{cache_verified_manifest, manifest_fingerprints, verified_cache_contains};
pub(super) use content_digest::model_content_sha256;
pub(super) use locks::open_or_create_model_lock;
pub(super) use publish::write_manifest;
#[cfg(feature = "local-onnx")]
pub(super) use transaction::ActiveRevisionTransaction;
pub(super) use transaction::{activation_pending, recover_pending_activation};

#[derive(Debug)]
pub(super) struct VerifiedLocalManifest {
    pub(super) manifest: LocalModelManifest,
    pub(super) artifact_sha256: String,
}

#[derive(Debug)]
pub(super) struct VerifiedCandidateManifest {
    artifact_sha256: String,
    #[cfg(feature = "local-onnx")]
    fingerprints: cache::ManifestFingerprints,
    #[cfg(feature = "local-onnx")]
    revision: String,
}

pub(super) fn collect_model_artifacts(
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
) -> Result<(Vec<LocalModelFile>, Vec<LocalModelSymlink>)> {
    artifacts::collect_model_artifacts(install_dir, preset)
}

pub(super) fn verify_unpublished_manifest(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<String> {
    Ok(verify_unpublished_candidate(install_dir, manifest, expected_preset)?.artifact_sha256)
}

pub(super) fn verify_unpublished_candidate(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<VerifiedCandidateManifest> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize {}", install_dir.display()))?;
    let preset = verify_manifest_header(manifest, expected_preset)?;
    let artifact_sha256 = model_content_sha256(manifest)?;
    let before = manifest_fingerprints(&canonical_install_dir, manifest)?;
    verify_runtime_layout(&canonical_install_dir, manifest, preset)?;
    verify_all_files(&canonical_install_dir, manifest)?;
    verify_runtime_layout(&canonical_install_dir, manifest, preset)?;
    let after = manifest_fingerprints(&canonical_install_dir, manifest)?;
    if before != after {
        bail!(
            "local embedding model files changed while verifying unpublished manifest in {}",
            canonical_install_dir.display()
        );
    }
    Ok(VerifiedCandidateManifest {
        artifact_sha256,
        #[cfg(feature = "local-onnx")]
        fingerprints: after,
        #[cfg(feature = "local-onnx")]
        revision: active_revision_from_manifest_paths(manifest, preset)?,
    })
}

#[cfg(feature = "local-onnx")]
pub(super) fn verify_imported_candidate(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    expected_preset: LocalEmbeddingPreset,
    revision: &str,
) -> Result<VerifiedCandidateManifest> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize {}", install_dir.display()))?;
    let preset = verify_manifest_header(manifest, Some(expected_preset))?;
    verify_candidate_ref_binding(manifest, preset, revision)?;
    let artifact_sha256 = model_content_sha256(manifest)?;
    let immutable = manifest_without_active_ref(manifest, preset);
    let before = manifest_fingerprints(&canonical_install_dir, &immutable)?;
    verify_runtime_layout_at_revision(manifest, preset, revision)?;
    verify_all_files(&canonical_install_dir, &immutable)?;
    verify_runtime_layout_at_revision(manifest, preset, revision)?;
    let after = manifest_fingerprints(&canonical_install_dir, &immutable)?;
    if before != after {
        bail!(
            "imported local embedding candidate changed while verifying in {}",
            canonical_install_dir.display()
        );
    }
    Ok(VerifiedCandidateManifest {
        artifact_sha256,
        fingerprints: after,
        revision: revision.to_string(),
    })
}

#[cfg(feature = "local-onnx")]
pub(super) fn verify_candidate_unchanged(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    expected_preset: LocalEmbeddingPreset,
    expected: &VerifiedCandidateManifest,
) -> Result<String> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize {}", install_dir.display()))?;
    let preset = verify_manifest_header(manifest, Some(expected_preset))?;
    let artifact_sha256 = model_content_sha256(manifest)?;
    if artifact_sha256 != expected.artifact_sha256 {
        bail!("prepared local model content identity changed before publish");
    }
    verify_candidate_ref_binding(manifest, preset, &expected.revision)?;
    verify_runtime_layout(&canonical_install_dir, manifest, preset)?;
    let active_ref_relative = format!("{}/refs/main", preset.cache_repo_dir());
    let active_ref = manifest
        .files
        .iter()
        .find(|file| file.path == active_ref_relative)
        .context("candidate manifest is missing active revision ref")?;
    verify_manifest_file(&canonical_install_dir, active_ref)?;
    let immutable = manifest_without_active_ref(manifest, preset);
    let current = manifest_fingerprints(&canonical_install_dir, &immutable)?;
    if current != expected.fingerprints {
        bail!("prepared local model files changed before publish");
    }
    verify_runtime_layout(&canonical_install_dir, manifest, preset)?;
    Ok(artifact_sha256)
}

#[cfg(feature = "local-onnx")]
fn manifest_without_active_ref(
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
) -> LocalModelManifest {
    let active_ref_relative = format!("{}/refs/main", preset.cache_repo_dir());
    let mut immutable = manifest.clone();
    immutable
        .files
        .retain(|file| file.path != active_ref_relative);
    immutable
}

#[cfg(feature = "local-onnx")]
fn verify_candidate_ref_binding(
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
    revision: &str,
) -> Result<()> {
    let active_ref_relative = format!("{}/refs/main", preset.cache_repo_dir());
    let active_ref = manifest
        .files
        .iter()
        .find(|file| file.path == active_ref_relative)
        .context("candidate manifest is missing active revision ref")?;
    let revision_bytes = revision.as_bytes();
    if active_ref.bytes != revision_bytes.len() as u64
        || active_ref.sha256 != sha256_bytes(revision_bytes)
    {
        bail!("candidate manifest active revision ref does not bind revision {revision}");
    }
    verify_runtime_layout_at_revision(manifest, preset, revision)
}

#[cfg(feature = "local-onnx")]
fn active_revision_from_manifest_paths(
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
) -> Result<String> {
    let prefix = format!("{}/snapshots/", preset.cache_repo_dir());
    let path = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .chain(
            manifest
                .symlinks
                .iter()
                .map(|symlink| symlink.path.as_str()),
        )
        .find(|path| path.starts_with(&prefix))
        .context("candidate manifest has no runtime snapshot path")?;
    let remainder = path
        .strip_prefix(&prefix)
        .context("candidate runtime path prefix changed")?;
    let revision = remainder
        .split_once('/')
        .map(|(revision, _)| revision)
        .context("candidate runtime path has no revision")?;
    Ok(revision.to_string())
}

#[cfg(feature = "local-onnx")]
pub(super) fn verified_runtime_file<'a>(
    install_dir: &Path,
    manifest: &'a LocalModelManifest,
    preset: LocalEmbeddingPreset,
    runtime_file: &str,
) -> Result<(&'a LocalModelFile, PathBuf)> {
    artifacts::verified_runtime_file(install_dir, manifest, preset, runtime_file)
}

pub(super) fn with_model_read_lock<T>(
    install_dir: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let (lock_path, read_lock) =
        open_or_create_model_lock(install_dir, super::MODEL_STATE_LOCK_FILE)
            .context("open local model state lock")?;
    fs2::FileExt::lock_shared(&read_lock)
        .with_context(|| format!("lock local model for reading {}", lock_path.display()))?;
    if activation_pending(install_dir)? {
        fs2::FileExt::unlock(&read_lock)
            .with_context(|| format!("unlock local model state {}", lock_path.display()))?;
        fs2::FileExt::lock_exclusive(&read_lock).with_context(|| {
            format!(
                "lock local model state for activation recovery {}",
                lock_path.display()
            )
        })?;
        recover_pending_activation(install_dir)?;
        fs2::FileExt::unlock(&read_lock).with_context(|| {
            format!("unlock recovered local model state {}", lock_path.display())
        })?;
        fs2::FileExt::lock_shared(&read_lock)
            .with_context(|| format!("relock local model for reading {}", lock_path.display()))?;
    }
    operation()
}

pub(super) fn read_verified_manifest(
    install_dir: &Path,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<VerifiedLocalManifest> {
    with_model_read_lock(install_dir, || {
        read_verified_manifest_unlocked(install_dir, expected_preset)
    })
}

pub(super) fn read_verified_manifest_compatible(
    install_dir: &Path,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<VerifiedLocalManifest> {
    let initial_error = match read_verified_manifest(install_dir, expected_preset) {
        Ok(verified) => return Ok(verified),
        Err(error) => error,
    };
    match declared_manifest_schema_version(install_dir) {
        Ok(1) => legacy::upgrade_schema_v1_manifest(install_dir, expected_preset),
        Ok(_) => read_verified_manifest(install_dir, expected_preset).or(Err(initial_error)),
        Err(_) => Err(initial_error),
    }
}

pub(super) fn read_verified_manifest_unlocked(
    install_dir: &Path,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<VerifiedLocalManifest> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize {}", install_dir.display()))?;
    let (path, _) = canonical_regular_path(&canonical_install_dir, MANIFEST_FILE)
        .context("verify local embedding manifest path")?;
    let content = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let manifest: LocalModelManifest =
        serde_json::from_slice(&content).with_context(|| format!("parse {}", path.display()))?;
    let preset = verify_manifest_header(&manifest, expected_preset)?;

    let manifest_sha256 = sha256_bytes(&content);
    let artifact_sha256 = model_content_sha256(&manifest)?;
    let before = manifest_fingerprints(&canonical_install_dir, &manifest)?;
    verify_runtime_layout(&canonical_install_dir, &manifest, preset)?;
    if verified_cache_contains(&canonical_install_dir, &manifest_sha256, &before)? {
        return Ok(VerifiedLocalManifest {
            manifest,
            artifact_sha256,
        });
    }

    verify_all_files(&canonical_install_dir, &manifest)?;
    verify_runtime_layout(&canonical_install_dir, &manifest, preset)?;
    let after = manifest_fingerprints(&canonical_install_dir, &manifest)?;
    if before != after {
        bail!(
            "local embedding model files changed while verifying {}",
            canonical_install_dir.display()
        );
    }
    cache_verified_manifest(&canonical_install_dir, &manifest_sha256, after)?;
    Ok(VerifiedLocalManifest {
        manifest,
        artifact_sha256,
    })
}

fn verify_manifest_header(
    manifest: &LocalModelManifest,
    expected_preset: Option<LocalEmbeddingPreset>,
) -> Result<LocalEmbeddingPreset> {
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
    if manifest.upstream_model != preset.upstream_model() {
        bail!(
            "manifest upstream_model {} does not match preset {} upstream {}",
            manifest.upstream_model,
            preset.label(),
            preset.upstream_model()
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
    let expected_source_url = preset.source_url();
    if manifest.source_url.as_deref() != Some(expected_source_url.as_str()) {
        bail!(
            "manifest source_url {} does not match preset {} source {}",
            manifest.source_url.as_deref().unwrap_or("<missing>"),
            preset.label(),
            expected_source_url
        );
    }
    if manifest.files.is_empty() {
        bail!("local embedding manifest has no verified files");
    }
    ensure_sorted_unique_paths(manifest.files.iter().map(|file| file.path.as_str()), "file")?;
    ensure_sorted_unique_paths(
        manifest
            .symlinks
            .iter()
            .map(|symlink| symlink.path.as_str()),
        "symlink",
    )?;
    let file_paths = manifest
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    for symlink in &manifest.symlinks {
        if file_paths.contains(symlink.path.as_str()) {
            bail!(
                "manifest path is declared as both file and symlink: {}",
                symlink.path
            );
        }
        if !file_paths.contains(symlink.resolved_path.as_str()) {
            bail!(
                "manifest symlink {} resolves to unlisted file {}",
                symlink.path,
                symlink.resolved_path
            );
        }
    }
    Ok(preset)
}

fn verify_all_files(install_dir: &Path, manifest: &LocalModelManifest) -> Result<()> {
    for file in &manifest.files {
        verify_manifest_file(install_dir, file)?;
    }
    for symlink in &manifest.symlinks {
        verify_manifest_symlink(install_dir, manifest, symlink)?;
    }
    Ok(())
}

pub(super) fn verify_manifest_file(install_dir: &Path, file: &LocalModelFile) -> Result<()> {
    let (path, metadata) = canonical_regular_path(install_dir, &file.path)?;
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

pub(super) fn verify_manifest_symlink(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    symlink: &LocalModelSymlink,
) -> Result<()> {
    let path = install_dir.join(checked_relative_path(&symlink.path)?);
    let metadata =
        std::fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    if !metadata.file_type().is_symlink() {
        bail!("manifest symlink path is not a symlink: {}", path.display());
    }
    let link_target = std::fs::read_link(&path)
        .with_context(|| format!("read symlink target {}", path.display()))?;
    let link_target = link_target.to_str().with_context(|| {
        format!(
            "local model symlink target is not Unicode: {}",
            path.display()
        )
    })?;
    if link_target != symlink.link_target {
        bail!(
            "local model symlink {} changed target: expected {}, got {}",
            symlink.path,
            symlink.link_target,
            link_target
        );
    }
    let resolved = resolve_relative_symlink(install_dir, &path, Path::new(link_target))?;
    let resolved_relative = relative_path_string(install_dir, &resolved)?;
    if resolved_relative != symlink.resolved_path {
        bail!(
            "local model symlink {} resolves to {}, expected {}",
            symlink.path,
            resolved_relative,
            symlink.resolved_path
        );
    }
    if !manifest
        .files
        .iter()
        .any(|file| file.path == resolved_relative)
    {
        bail!(
            "local model symlink {} resolves to unlisted file {}",
            symlink.path,
            resolved_relative
        );
    }
    Ok(())
}

fn ensure_sorted_unique_paths<'a>(paths: impl Iterator<Item = &'a str>, kind: &str) -> Result<()> {
    let paths = paths.collect::<Vec<_>>();
    let mut seen = HashSet::new();
    for path in &paths {
        checked_relative_path(path)?;
        if !seen.insert(*path) {
            bail!("duplicate local embedding manifest {kind} path: {path}");
        }
    }
    if paths.windows(2).any(|pair| pair[0] > pair[1]) {
        bail!("local embedding manifest {kind} paths are not sorted");
    }
    Ok(())
}

fn sha256_bytes(content: &[u8]) -> String {
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn declared_manifest_schema_version(install_dir: &Path) -> Result<u32> {
    #[derive(serde::Deserialize)]
    struct ManifestVersion {
        schema_version: u32,
    }

    let (path, _) = canonical_regular_path(install_dir, MANIFEST_FILE)
        .context("verify declared local embedding manifest path")?;
    let content = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    let version: ManifestVersion =
        serde_json::from_slice(&content).with_context(|| format!("parse {}", path.display()))?;
    Ok(version.schema_version)
}

#[cfg(test)]
mod tests;
