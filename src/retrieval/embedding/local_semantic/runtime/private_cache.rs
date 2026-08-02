#[cfg(unix)]
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};

#[cfg(not(windows))]
use super::super::fs_cleanup::remove_managed_tree;
use super::super::manifest::verified_runtime_file;
use super::super::{
    checked_relative_path, sha256_file, LocalEmbeddingPreset, LocalModelFile, LocalModelManifest,
};

#[path = "private_cache/layout.rs"]
mod layout;
#[path = "private_cache/locks.rs"]
mod locks;
#[cfg(test)]
#[path = "private_cache/tests.rs"]
mod tests;

#[cfg(windows)]
use crate::retrieval::embedding::local_semantic::{
    windows_cleanup,
    windows_security::{self, DirectoryAnchor},
};
use layout::{PrivateRuntimeFile, PrivateRuntimeLayout};
use locks::{
    exclusive_lock, release_and_remove_usage_lock, shared_lock, try_exclusive_lock,
    usage_lock_name, CacheLock,
};

const PRIVATE_RUNTIME_CACHE_DIR: &str = ".remem-private-runtime-cache";
const PRIVATE_RUNTIME_LAYOUT_VERSION: &str = "v1";
const MATERIALIZE_LOCK_FILE: &str = ".materialize.lock";
static PRIVATE_RUNTIME_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct SourceRuntimeFile {
    private: PrivateRuntimeFile,
    source_path: PathBuf,
}

#[derive(Debug)]
pub(super) struct PrivateRuntimeCache {
    layout: PrivateRuntimeLayout,
    #[cfg(windows)]
    _directory_anchors: WindowsPrivateCacheAnchors,
    _usage_lock: CacheLock,
}

#[cfg(windows)]
#[derive(Debug)]
struct WindowsPrivateCacheAnchors {
    _install: DirectoryAnchor,
    _cache_parent: DirectoryAnchor,
    _runtime: DirectoryAnchor,
}

#[cfg(windows)]
impl WindowsPrivateCacheAnchors {
    fn new(parent: PreparedCacheParent, runtime: DirectoryAnchor) -> Self {
        Self {
            _install: parent.install_anchor,
            _cache_parent: parent.cache_parent_anchor,
            _runtime: runtime,
        }
    }
}

impl PrivateRuntimeCache {
    pub(super) fn materialize(
        install_dir: &Path,
        manifest: &LocalModelManifest,
        preset: LocalEmbeddingPreset,
        artifact_sha256: &str,
    ) -> Result<Self> {
        validate_artifact_sha256(artifact_sha256)?;
        let canonical_install_dir = std::fs::canonicalize(install_dir)
            .with_context(|| format!("canonicalize local model dir {}", install_dir.display()))?;
        let prepared_parent = prepare_cache_parent(&canonical_install_dir)?;
        let cache_parent = prepared_parent.path.clone();
        let _materialize_lock = exclusive_lock(&cache_parent, MATERIALIZE_LOCK_FILE)?;
        let cache_name = deterministic_cache_name(preset, artifact_sha256);
        cleanup_stale_caches(&cache_parent, &cache_name)?;
        let sources =
            collect_runtime_sources(&canonical_install_dir, manifest, preset, artifact_sha256)?;
        let published_path = cache_parent.join(&cache_name);
        let published_layout = runtime_layout(&published_path, preset, artifact_sha256, &sources);

        if existing_cache_directory(&cache_parent, &published_path)? {
            #[cfg(windows)]
            let published_anchor = DirectoryAnchor::open_owner_only(&published_path, false)
                .with_context(|| {
                    format!(
                        "anchor existing private runtime cache {}",
                        published_path.display()
                    )
                })?;
            let usage_lock = shared_lock(&cache_parent, &usage_lock_name(&cache_name))?;
            match published_layout.verify() {
                Ok(()) => {
                    return Ok(Self {
                        layout: published_layout,
                        #[cfg(windows)]
                        _directory_anchors: WindowsPrivateCacheAnchors::new(
                            prepared_parent,
                            published_anchor,
                        ),
                        _usage_lock: usage_lock,
                    });
                }
                Err(verify_error) => {
                    drop(usage_lock);
                    let Some(exclusive_usage) =
                        try_exclusive_lock(&cache_parent, &usage_lock_name(&cache_name))?
                    else {
                        bail!(
                            "verified private runtime cache {} is corrupt and currently in use: {verify_error:#}",
                            published_path.display()
                        );
                    };
                    #[cfg(windows)]
                    windows_cleanup::remove_existing_owner_only_tree(
                        &published_path,
                        published_anchor,
                    )?;
                    #[cfg(not(windows))]
                    remove_managed_tree(&cache_parent, &published_path).with_context(|| {
                        format!(
                            "remove corrupt private runtime cache {}",
                            published_path.display()
                        )
                    })?;
                    release_and_remove_usage_lock(&cache_parent, &cache_name, exclusive_usage)?;
                }
            }
        }

        let mut staging_cleanup = create_unique_staging_dir(&cache_parent, &cache_name)?;
        let staging_path = staging_cleanup.path().to_path_buf();
        let repo_dir = PathBuf::from(preset.cache_repo_dir());
        let ref_relative = repo_dir.join("refs/main");
        let ref_path = staging_path.join(&ref_relative);
        std::fs::create_dir_all(
            ref_path
                .parent()
                .context("private runtime ref should have a parent")?,
        )?;
        std::fs::write(&ref_path, artifact_sha256)
            .with_context(|| format!("write private runtime ref {}", ref_path.display()))?;
        make_file_read_only(&ref_path)?;

        for source in &sources {
            let destination_path = staging_path.join(&source.private.relative_path);
            std::fs::create_dir_all(
                destination_path
                    .parent()
                    .context("private runtime artifact should have a parent")?,
            )?;
            clone_or_copy_independent_file(
                &source.source_path,
                &destination_path,
                &source.private.expected,
            )?;
        }

        let staged = PrivateRuntimeLayout {
            root: staging_path.clone(),
            repo_dir: repo_dir.clone(),
            revision: artifact_sha256.to_string(),
            files: sources
                .iter()
                .map(|source| source.private.clone())
                .collect(),
        };
        staged
            .verify()
            .context("verify staged private local embedding runtime cache")?;
        if std::fs::symlink_metadata(&published_path).is_ok() {
            bail!(
                "private runtime publish path appeared while materializing: {}",
                published_path.display()
            );
        }
        #[cfg(windows)]
        let published_anchor = staging_cleanup.publish(&published_path)?;
        #[cfg(not(windows))]
        {
            std::fs::rename(&staging_path, &published_path).with_context(|| {
                format!(
                    "atomically publish private local embedding runtime cache {}",
                    published_path.display()
                )
            })?;
            staging_cleanup.disarm();
        }
        sync_parent_dir(&cache_parent)?;

        let published = PrivateRuntimeLayout {
            root: published_path,
            repo_dir,
            revision: artifact_sha256.to_string(),
            files: staged.files,
        };
        published
            .verify()
            .context("verify published private local embedding runtime cache")?;
        let usage_lock = shared_lock(&cache_parent, &usage_lock_name(&cache_name))?;
        Ok(Self {
            layout: published,
            #[cfg(windows)]
            _directory_anchors: WindowsPrivateCacheAnchors::new(prepared_parent, published_anchor),
            _usage_lock: usage_lock,
        })
    }

    pub(super) fn root(&self) -> &Path {
        &self.layout.root
    }

    pub(super) fn verify(&self) -> Result<()> {
        self.layout.verify()
    }
}

struct RuntimeDirectoryCleanup {
    parent: PathBuf,
    path: PathBuf,
    #[cfg(windows)]
    staging_anchor: Option<DirectoryAnchor>,
    armed: bool,
}

impl RuntimeDirectoryCleanup {
    fn new(parent: PathBuf, path: PathBuf, created: CreatedStagingDirectory) -> Self {
        #[cfg(not(windows))]
        let CreatedStagingDirectory {} = created;
        Self {
            parent,
            path,
            #[cfg(windows)]
            staging_anchor: Some(created.anchor),
            armed: true,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(not(windows))]
    fn disarm(&mut self) {
        self.armed = false;
    }

    #[cfg(windows)]
    fn publish(&mut self, published_path: &Path) -> Result<DirectoryAnchor> {
        let staging = self
            .staging_anchor
            .take()
            .context("Windows runtime staging anchor is missing")?;
        // Once rename begins, an error must leave the object for the next
        // verified stale-cleanup pass instead of deleting a replacement path.
        self.armed = false;
        windows_cleanup::publish_identity_handoff(&self.path, published_path, staging)
    }
}

impl Drop for RuntimeDirectoryCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if self.path.parent() != Some(self.parent.as_path()) {
            crate::log::error(
                "embedding",
                &format!(
                    "refusing private runtime staging cleanup outside its parent: {}",
                    self.path.display()
                ),
            );
            return;
        }
        #[cfg(windows)]
        let result = self
            .staging_anchor
            .take()
            .context("refusing Windows runtime staging cleanup without its FileId anchor")
            .and_then(|anchor| windows_cleanup::remove_renameable_staging(&self.path, anchor));
        #[cfg(not(windows))]
        let result = remove_managed_tree(&self.parent, &self.path);
        if let Err(error) = result {
            crate::log::error(
                "embedding",
                &format!(
                    "clean failed private local embedding runtime staging dir {}: {error:#}",
                    self.path.display()
                ),
            );
        }
    }
}

struct PreparedCacheParent {
    path: PathBuf,
    #[cfg(windows)]
    install_anchor: DirectoryAnchor,
    #[cfg(windows)]
    cache_parent_anchor: DirectoryAnchor,
}

fn prepare_cache_parent(install_dir: &Path) -> Result<PreparedCacheParent> {
    #[cfg(windows)]
    let install_anchor =
        DirectoryAnchor::open_owner_only(install_dir, false).with_context(|| {
            format!(
                "validate and anchor owner-only model dir {}",
                install_dir.display()
            )
        })?;
    let parent = install_dir.join(PRIVATE_RUNTIME_CACHE_DIR);
    #[cfg(not(windows))]
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("create private runtime cache parent {}", parent.display()))?;
    #[cfg(windows)]
    let cache_parent_anchor = windows_security::open_or_create_owner_only_directory(&parent)
        .with_context(|| {
            format!(
                "create or anchor private runtime cache parent {}",
                parent.display()
            )
        })?;
    let metadata = std::fs::symlink_metadata(&parent)
        .with_context(|| format!("stat private runtime cache parent {}", parent.display()))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "private runtime cache parent is not a directory: {}",
            parent.display()
        );
    }
    let canonical_parent = std::fs::canonicalize(&parent).with_context(|| {
        format!(
            "canonicalize private runtime cache parent {}",
            parent.display()
        )
    })?;
    if canonical_parent != parent || !canonical_parent.starts_with(install_dir) {
        bail!(
            "private runtime cache parent escapes verified model directory: {}",
            canonical_parent.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("secure private runtime cache parent {}", parent.display()))?;
    }
    #[cfg(windows)]
    {
        install_anchor
            .verify_path()
            .with_context(|| format!("revalidate local model dir {}", install_dir.display()))?;
        cache_parent_anchor.verify_path().with_context(|| {
            format!(
                "revalidate private runtime cache parent {}",
                parent.display()
            )
        })?;
    }
    Ok(PreparedCacheParent {
        path: parent,
        #[cfg(windows)]
        install_anchor,
        #[cfg(windows)]
        cache_parent_anchor,
    })
}

struct CreatedStagingDirectory {
    #[cfg(windows)]
    anchor: DirectoryAnchor,
}

fn create_unique_staging_dir(parent: &Path, cache_name: &str) -> Result<RuntimeDirectoryCleanup> {
    for _ in 0..64 {
        let sequence = PRIVATE_RUNTIME_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let staging = parent.join(format!(
            ".{cache_name}.tmp-{}-{sequence}",
            std::process::id()
        ));
        match create_owner_only_directory(&staging) {
            Ok(created) => {
                return Ok(RuntimeDirectoryCleanup::new(
                    parent.to_path_buf(),
                    staging,
                    created,
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "create private local embedding runtime staging dir {}",
                        staging.display()
                    )
                })
            }
        }
    }
    bail!(
        "could not allocate a unique private runtime cache under {}",
        parent.display()
    )
}

#[cfg(unix)]
fn create_owner_only_directory(path: &Path) -> std::io::Result<CreatedStagingDirectory> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700).create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(CreatedStagingDirectory {})
}

#[cfg(windows)]
fn create_owner_only_directory(path: &Path) -> std::io::Result<CreatedStagingDirectory> {
    windows_security::create_owner_only_directory(path, true)
        .map(|anchor| CreatedStagingDirectory { anchor })
}

#[cfg(not(any(unix, windows)))]
fn create_owner_only_directory(path: &Path) -> std::io::Result<CreatedStagingDirectory> {
    std::fs::create_dir(path)?;
    Ok(CreatedStagingDirectory {})
}

fn deterministic_cache_name(preset: LocalEmbeddingPreset, artifact_sha256: &str) -> String {
    format!(
        "{PRIVATE_RUNTIME_LAYOUT_VERSION}-{}-{artifact_sha256}",
        preset.label()
    )
}

fn collect_runtime_sources(
    install_dir: &Path,
    manifest: &LocalModelManifest,
    preset: LocalEmbeddingPreset,
    artifact_sha256: &str,
) -> Result<Vec<SourceRuntimeFile>> {
    let snapshot_relative = PathBuf::from(preset.cache_repo_dir())
        .join("snapshots")
        .join(artifact_sha256);
    preset
        .required_runtime_files()
        .map(|runtime_file| {
            let (expected, source_path) =
                verified_runtime_file(install_dir, manifest, preset, runtime_file)?;
            Ok(SourceRuntimeFile {
                private: PrivateRuntimeFile {
                    relative_path: snapshot_relative.join(checked_relative_path(runtime_file)?),
                    expected: expected.clone(),
                },
                source_path,
            })
        })
        .collect()
}

fn runtime_layout(
    root: &Path,
    preset: LocalEmbeddingPreset,
    artifact_sha256: &str,
    sources: &[SourceRuntimeFile],
) -> PrivateRuntimeLayout {
    PrivateRuntimeLayout {
        root: root.to_path_buf(),
        repo_dir: PathBuf::from(preset.cache_repo_dir()),
        revision: artifact_sha256.to_string(),
        files: sources
            .iter()
            .map(|source| source.private.clone())
            .collect(),
    }
}

fn existing_cache_directory(parent: &Path, path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("stat private runtime cache {}", path.display()))
        }
        Ok(metadata) if !metadata.file_type().is_dir() => {
            bail!(
                "private runtime cache path is not a regular directory: {}",
                path.display()
            )
        }
        Ok(_) => {
            let canonical = std::fs::canonicalize(path).with_context(|| {
                format!("canonicalize private runtime cache {}", path.display())
            })?;
            if canonical != path || !canonical.starts_with(parent) {
                bail!(
                    "private runtime cache path escapes cache parent: {}",
                    canonical.display()
                );
            }
            Ok(true)
        }
    }
}

fn cleanup_stale_caches(parent: &Path, current_cache_name: &str) -> Result<()> {
    let entries = std::fs::read_dir(parent)
        .with_context(|| format!("read private runtime cache parent {}", parent.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if is_managed_staging_name(&name) {
            remove_stale_staging(parent, &entry.path())?;
        } else if is_managed_cache_name(&name) && name != current_cache_name {
            remove_stale_cache_if_unused(parent, &name)?;
        }
    }
    cleanup_orphan_usage_locks(parent, current_cache_name)
}

fn remove_stale_staging(parent: &Path, path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("stat stale private runtime staging {}", path.display()))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "managed private runtime staging path is not a directory: {}",
            path.display()
        );
    }
    let canonical = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize stale runtime staging {}", path.display()))?;
    if canonical != path || !canonical.starts_with(parent) {
        bail!(
            "stale private runtime staging escapes cache parent: {}",
            canonical.display()
        );
    }
    #[cfg(windows)]
    let result = windows_cleanup::remove_stale_staging(path);
    #[cfg(not(windows))]
    let result = remove_managed_tree(parent, path);
    result.with_context(|| format!("remove stale private runtime staging {}", path.display()))
}

fn remove_stale_cache_if_unused(parent: &Path, cache_name: &str) -> Result<()> {
    let lock_name = usage_lock_name(cache_name);
    let Some(exclusive_usage) = try_exclusive_lock(parent, &lock_name)? else {
        return Ok(());
    };
    let cache_path = parent.join(cache_name);
    if existing_cache_directory(parent, &cache_path)? {
        #[cfg(windows)]
        let cache_anchor =
            DirectoryAnchor::open_owner_only(&cache_path, false).with_context(|| {
                format!(
                    "anchor stale private runtime cache {}",
                    cache_path.display()
                )
            })?;
        #[cfg(windows)]
        windows_cleanup::remove_existing_owner_only_tree(&cache_path, cache_anchor)?;
        #[cfg(not(windows))]
        remove_managed_tree(parent, &cache_path).with_context(|| {
            format!(
                "remove stale private runtime cache {}",
                cache_path.display()
            )
        })?;
    }
    release_and_remove_usage_lock(parent, cache_name, exclusive_usage)
}

fn cleanup_orphan_usage_locks(parent: &Path, current_cache_name: &str) -> Result<()> {
    let entries = std::fs::read_dir(parent)
        .with_context(|| format!("read private runtime cache locks {}", parent.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(cache_name) = cache_name_from_usage_lock(&name) else {
            continue;
        };
        if cache_name == current_cache_name {
            continue;
        }
        match std::fs::symlink_metadata(parent.join(&cache_name)) {
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect cache for orphan private runtime lock {name}")
                })
            }
        }
        if let Some(exclusive_usage) = try_exclusive_lock(parent, &name)? {
            release_and_remove_usage_lock(parent, &cache_name, exclusive_usage)?;
        }
    }
    Ok(())
}

fn is_managed_cache_name(name: &str) -> bool {
    LocalEmbeddingPreset::all().iter().copied().any(|preset| {
        let prefix = format!("{PRIVATE_RUNTIME_LAYOUT_VERSION}-{}-", preset.label());
        name.strip_prefix(&prefix).is_some_and(is_sha256)
    })
}

fn is_managed_staging_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some((cache_name, nonce)) = rest.split_once(".tmp-") else {
        return false;
    };
    is_managed_cache_name(cache_name)
        && nonce
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn cache_name_from_usage_lock(name: &str) -> Option<String> {
    let cache_name = name.strip_prefix('.')?.strip_suffix(".usage.lock")?;
    is_managed_cache_name(cache_name).then(|| cache_name.to_string())
}

fn clone_or_copy_independent_file(
    source: &Path,
    destination: &Path,
    expected: &LocalModelFile,
) -> Result<()> {
    let source_metadata = std::fs::symlink_metadata(source)
        .with_context(|| format!("stat verified runtime source {}", source.display()))?;
    if !source_metadata.file_type().is_file() || source_metadata.len() != expected.bytes {
        bail!(
            "verified runtime source {} changed before private copy",
            source.display()
        );
    }

    if !try_clone_file(source, destination)? {
        let copied = std::fs::copy(source, destination).with_context(|| {
            format!(
                "copy verified runtime artifact {} to {}",
                source.display(),
                destination.display()
            )
        })?;
        if copied != expected.bytes {
            bail!(
                "private runtime copy {} wrote {} bytes, expected {}",
                destination.display(),
                copied,
                expected.bytes
            );
        }
    }

    let destination_metadata = std::fs::symlink_metadata(destination)
        .with_context(|| format!("stat private runtime copy {}", destination.display()))?;
    if !destination_metadata.file_type().is_file() {
        bail!(
            "private runtime copy is not a regular file: {}",
            destination.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if source_metadata.dev() == destination_metadata.dev()
            && source_metadata.ino() == destination_metadata.ino()
        {
            bail!(
                "private runtime artifact {} is a hardlink to mutable source {}",
                destination.display(),
                source.display()
            );
        }
    }
    let actual = sha256_file(destination)?;
    if destination_metadata.len() != expected.bytes || actual != expected.sha256 {
        bail!(
            "private runtime artifact {} failed size/checksum verification",
            destination.display()
        );
    }
    make_file_read_only(destination)?;
    Ok(())
}

fn make_file_read_only(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("stat private runtime file {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!(
            "private runtime path is not a regular file: {}",
            path.display()
        );
    }
    let mut permissions = metadata.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(path, permissions)
        .with_context(|| format!("make private runtime file read-only {}", path.display()))?;
    Ok(())
}

fn validate_artifact_sha256(value: &str) -> Result<()> {
    if !is_sha256(value) {
        bail!("invalid local model artifact SHA-256 for private runtime cache");
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> Result<()> {
    File::open(parent)
        .with_context(|| format!("open private runtime cache parent {}", parent.display()))?
        .sync_all()
        .with_context(|| format!("sync private runtime cache parent {}", parent.display()))
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn try_clone_file(source: &Path, destination: &Path) -> Result<bool> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .context("source path for clonefile contains NUL")?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .context("destination path for clonefile contains NUL")?;
    // SAFETY: both C strings are NUL-terminated and valid for the duration of the call.
    let result = unsafe { libc::clonefile(source.as_ptr(), destination.as_ptr(), 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ENOTSUP | libc::EXDEV | libc::EINVAL | libc::ENOSYS) => Ok(false),
        _ => Err(error).context("clone verified runtime artifact with clonefile"),
    }
}

#[cfg(target_os = "linux")]
fn try_clone_file(source: &Path, destination: &Path) -> Result<bool> {
    use std::os::fd::AsRawFd;

    let source_file =
        std::fs::File::open(source).with_context(|| format!("open {}", source.display()))?;
    let destination_file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .with_context(|| format!("create {}", destination.display()))?;
    // SAFETY: both file descriptors remain open for the duration of the ioctl.
    let result = unsafe {
        libc::ioctl(
            destination_file.as_raw_fd(),
            libc::FICLONE as _,
            source_file.as_raw_fd(),
        )
    };
    if result == 0 {
        return Ok(true);
    }
    drop(destination_file);
    std::fs::remove_file(destination)
        .with_context(|| format!("remove failed reflink target {}", destination.display()))?;
    Ok(false)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn try_clone_file(_source: &Path, _destination: &Path) -> Result<bool> {
    Ok(false)
}
