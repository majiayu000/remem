use std::path::{Path, PathBuf};
use std::time::SystemTime;
#[cfg(any(unix, windows))]
use std::{collections::VecDeque, sync::Mutex, sync::OnceLock};

use anyhow::{bail, Context, Result};

#[cfg(windows)]
use super::super::windows_security::{self, WindowsPathFingerprint};
use super::super::{checked_relative_path, LocalModelFile, LocalModelManifest, LocalModelSymlink};
use super::artifacts::canonical_regular_path;
use super::verify_manifest_symlink;

const VERIFIED_MANIFEST_CACHE_CAPACITY: usize = 8;

#[cfg(any(unix, windows))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedManifestCacheEntry {
    install_dir: PathBuf,
    manifest_sha256: String,
    fingerprints: ManifestFingerprints,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ManifestFingerprints {
    files: Vec<ModelFileFingerprint>,
    symlinks: Vec<ModelSymlinkFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelFileFingerprint {
    relative_path: String,
    metadata: MetadataFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelSymlinkFingerprint {
    relative_path: String,
    link_target: String,
    resolved_path: String,
    link_metadata: MetadataFingerprint,
    resolved_file: ModelFileFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MetadataFingerprint {
    bytes: u64,
    modified: SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    change_time_secs: i64,
    #[cfg(unix)]
    change_time_nanos: i64,
    #[cfg(windows)]
    windows: WindowsPathFingerprint,
}

#[cfg(any(unix, windows))]
static VERIFIED_MANIFEST_CACHE: OnceLock<Mutex<VecDeque<VerifiedManifestCacheEntry>>> =
    OnceLock::new();

pub(super) fn manifest_fingerprints(
    install_dir: &Path,
    manifest: &LocalModelManifest,
) -> Result<ManifestFingerprints> {
    let files = manifest
        .files
        .iter()
        .map(|file| model_file_fingerprint(install_dir, file))
        .collect::<Result<Vec<_>>>()?;
    let symlinks = manifest
        .symlinks
        .iter()
        .map(|symlink| model_symlink_fingerprint(install_dir, manifest, symlink))
        .collect::<Result<Vec<_>>>()?;
    Ok(ManifestFingerprints { files, symlinks })
}

fn model_file_fingerprint(
    install_dir: &Path,
    file: &LocalModelFile,
) -> Result<ModelFileFingerprint> {
    let (path, metadata) = canonical_regular_path(install_dir, &file.path)?;
    if metadata.len() != file.bytes {
        bail!(
            "checksum target {} size changed: expected {} bytes, got {}",
            path.display(),
            file.bytes,
            metadata.len()
        );
    }
    Ok(ModelFileFingerprint {
        relative_path: file.path.clone(),
        metadata: metadata_fingerprint(&path, &metadata)?,
    })
}

fn model_symlink_fingerprint(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    symlink: &LocalModelSymlink,
) -> Result<ModelSymlinkFingerprint> {
    verify_manifest_symlink(install_dir, manifest, symlink)?;
    let path = install_dir.join(checked_relative_path(&symlink.path)?);
    let metadata =
        std::fs::symlink_metadata(&path).with_context(|| format!("stat {}", path.display()))?;
    let resolved_file = manifest
        .files
        .iter()
        .find(|file| file.path == symlink.resolved_path)
        .with_context(|| {
            format!(
                "manifest symlink {} resolves to unlisted file {}",
                symlink.path, symlink.resolved_path
            )
        })?;
    Ok(ModelSymlinkFingerprint {
        relative_path: symlink.path.clone(),
        link_target: symlink.link_target.clone(),
        resolved_path: symlink.resolved_path.clone(),
        link_metadata: metadata_fingerprint(&path, &metadata)?,
        resolved_file: model_file_fingerprint(install_dir, resolved_file)?,
    })
}

fn metadata_fingerprint(path: &Path, metadata: &std::fs::Metadata) -> Result<MetadataFingerprint> {
    let modified = metadata
        .modified()
        .with_context(|| format!("read modified time for {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(MetadataFingerprint {
            bytes: metadata.len(),
            modified,
            device: metadata.dev(),
            inode: metadata.ino(),
            change_time_secs: metadata.ctime(),
            change_time_nanos: metadata.ctime_nsec(),
        })
    }
    #[cfg(windows)]
    {
        Ok(MetadataFingerprint {
            bytes: metadata.len(),
            modified,
            windows: windows_security::path_fingerprint(path, metadata.file_type().is_symlink())
                .with_context(|| format!("read Windows file identity for {}", path.display()))?,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(MetadataFingerprint {
            bytes: metadata.len(),
            modified,
        })
    }
}

#[cfg(any(unix, windows))]
pub(super) fn verified_cache_contains(
    install_dir: &Path,
    manifest_sha256: &str,
    fingerprints: &ManifestFingerprints,
) -> Result<bool> {
    let cache = verified_manifest_cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("local model verification cache lock poisoned"))?;
    Ok(cache.iter().any(|entry| {
        entry.install_dir == install_dir
            && entry.manifest_sha256 == manifest_sha256
            && entry.fingerprints == *fingerprints
    }))
}

#[cfg(not(any(unix, windows)))]
pub(super) fn verified_cache_contains(
    _install_dir: &Path,
    _manifest_sha256: &str,
    _fingerprints: &ManifestFingerprints,
) -> Result<bool> {
    Ok(false)
}

#[cfg(any(unix, windows))]
pub(super) fn cache_verified_manifest(
    install_dir: &Path,
    manifest_sha256: &str,
    fingerprints: ManifestFingerprints,
) -> Result<()> {
    let mut cache = verified_manifest_cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("local model verification cache lock poisoned"))?;
    cache.retain(|entry| {
        entry.install_dir != install_dir || entry.manifest_sha256 != manifest_sha256
    });
    cache.push_back(VerifiedManifestCacheEntry {
        install_dir: install_dir.to_path_buf(),
        manifest_sha256: manifest_sha256.to_string(),
        fingerprints,
    });
    while cache.len() > VERIFIED_MANIFEST_CACHE_CAPACITY {
        cache.pop_front();
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(super) fn cache_verified_manifest(
    _install_dir: &Path,
    _manifest_sha256: &str,
    _fingerprints: ManifestFingerprints,
) -> Result<()> {
    Ok(())
}

#[cfg(any(unix, windows))]
fn verified_manifest_cache() -> &'static Mutex<VecDeque<VerifiedManifestCacheEntry>> {
    VERIFIED_MANIFEST_CACHE.get_or_init(|| Mutex::new(VecDeque::new()))
}
