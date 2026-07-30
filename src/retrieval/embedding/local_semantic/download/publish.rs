use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use super::super::manifest::{
    canonical_regular_path, verify_candidate_unchanged, verify_imported_candidate,
    verify_manifest_file, verify_manifest_symlink, write_manifest, ActiveRevisionTransaction,
    VerifiedCandidateManifest, VerifiedLocalManifest,
};
use super::super::{
    checked_relative_path, LocalEmbeddingPreset, LocalModelFile, LocalModelManifest,
    LocalModelSymlink,
};
use super::validate_revision;

static PUBLISH_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub(in crate::retrieval::embedding::local_semantic) struct ImportedLocalModel {
    pub(in crate::retrieval::embedding::local_semantic) revision: String,
    pub(in crate::retrieval::embedding::local_semantic) verification: VerifiedCandidateManifest,
}

pub(in crate::retrieval::embedding::local_semantic) fn activate_candidate_manifest(
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
    manifest: LocalModelManifest,
    artifact_sha256: String,
    imported: ImportedLocalModel,
) -> Result<VerifiedLocalManifest> {
    let mut active_revision =
        ActiveRevisionTransaction::begin(install_dir, preset, &imported.revision, &manifest)?;
    let verified_sha256 =
        match verify_candidate_unchanged(install_dir, &manifest, preset, &imported.verification) {
            Ok(verified) => verified,
            Err(error) => return rollback_with_error(&mut active_revision, error),
        };
    if verified_sha256 != artifact_sha256 {
        return rollback_with_error(
            &mut active_revision,
            anyhow::anyhow!(
                "imported local model content identity changed for {}: prepared sha256:{artifact_sha256}, imported sha256:{verified_sha256}",
                preset.label()
            ),
        );
    }
    if let Err(error) = active_revision.mark_committing() {
        return rollback_with_error(&mut active_revision, error);
    }
    if let Err(error) = write_manifest(install_dir, &manifest) {
        return rollback_with_error(&mut active_revision, error);
    }
    active_revision.commit()?;
    Ok(VerifiedLocalManifest {
        manifest,
        artifact_sha256,
    })
}

fn rollback_with_error<T>(
    transaction: &mut ActiveRevisionTransaction,
    error: anyhow::Error,
) -> Result<T> {
    match transaction.resolve() {
        Ok(()) => Err(error),
        Err(rollback_error) => Err(anyhow::anyhow!(
            "{error:#}; additionally failed to resolve the active model transaction: {rollback_error:#}"
        )),
    }
}

pub(in crate::retrieval::embedding::local_semantic) fn import_immutable_candidate(
    staging_dir: &Path,
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
    manifest: &LocalModelManifest,
) -> Result<ImportedLocalModel> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize local model install {}", install_dir.display()))?;
    let active_ref_relative = format!("{}/refs/main", preset.cache_repo_dir());
    let active_ref = manifest
        .files
        .iter()
        .find(|file| file.path == active_ref_relative)
        .context("candidate manifest is missing active Hugging Face revision ref")?;
    let (staging_ref_path, _) = canonical_regular_path(staging_dir, &active_ref.path)?;
    let active_ref_bytes = std::fs::read(&staging_ref_path)
        .with_context(|| format!("read staged revision {}", staging_ref_path.display()))?;
    let revision_text = std::str::from_utf8(&active_ref_bytes)
        .context("staged Hugging Face revision is not UTF-8")?;
    let revision = validate_revision(revision_text)?;

    for file in manifest
        .files
        .iter()
        .filter(|file| file.path != active_ref_relative)
    {
        install_manifest_file(staging_dir, &canonical_install_dir, file)?;
    }
    for symlink in &manifest.symlinks {
        install_manifest_symlink(&canonical_install_dir, symlink)?;
    }
    let pinned_ref_relative = format!("{}/refs/{revision}", preset.cache_repo_dir());
    install_bytes_if_absent(
        &canonical_install_dir,
        &pinned_ref_relative,
        &active_ref_bytes,
    )?;

    for file in manifest
        .files
        .iter()
        .filter(|file| file.path != active_ref_relative)
    {
        verify_manifest_file(&canonical_install_dir, file)
            .with_context(|| format!("verify imported immutable file {}", file.path))?;
    }
    for symlink in &manifest.symlinks {
        verify_manifest_symlink(&canonical_install_dir, manifest, symlink)
            .with_context(|| format!("verify imported immutable symlink {}", symlink.path))?;
    }
    let verification =
        verify_imported_candidate(&canonical_install_dir, manifest, preset, &revision)
            .context("verify fully imported immutable local model candidate")?;
    Ok(ImportedLocalModel {
        revision,
        verification,
    })
}

fn install_manifest_file(
    staging_dir: &Path,
    install_dir: &Path,
    expected: &LocalModelFile,
) -> Result<()> {
    let target = target_path_with_real_parent(install_dir, &expected.path)?;
    match std::fs::symlink_metadata(&target) {
        Ok(_) => return verify_manifest_file(install_dir, expected),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("stat import target {}", target.display()));
        }
    }
    let (source, source_metadata) = canonical_regular_path(staging_dir, &expected.path)?;
    if source_metadata.len() != expected.bytes {
        bail!(
            "staged model file {} size changed before import: expected {}, got {}",
            expected.path,
            expected.bytes,
            source_metadata.len()
        );
    }
    let (temp_path, mut temp_file) = create_publish_temp(&target)?;
    let mut cleanup = TempFileCleanup::new(temp_path.clone());
    let mut source_file =
        File::open(&source).with_context(|| format!("open staged file {}", source.display()))?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = source_file
            .read(&mut buffer)
            .with_context(|| format!("read staged file {}", source.display()))?;
        if read == 0 {
            break;
        }
        temp_file
            .write_all(&buffer[..read])
            .with_context(|| format!("write imported temp {}", temp_path.display()))?;
        hasher.update(&buffer[..read]);
        bytes = bytes
            .checked_add(read as u64)
            .context("count imported local model bytes")?;
    }
    let sha256 = hex_digest(hasher.finalize());
    if bytes != expected.bytes || sha256 != expected.sha256 {
        bail!(
            "staged model file {} changed while importing: expected {} bytes sha256:{}, got {bytes} bytes sha256:{sha256}",
            expected.path,
            expected.bytes,
            expected.sha256
        );
    }
    temp_file
        .sync_all()
        .with_context(|| format!("sync imported temp {}", temp_path.display()))?;
    drop(temp_file);
    let outcome = publish_no_clobber(&temp_path, &target).with_context(|| {
        format!(
            "publish immutable local model file {} from {}",
            target.display(),
            temp_path.display()
        )
    })?;
    if outcome.target_preexisted {
        verify_manifest_file(install_dir, expected)?;
    }
    finish_publish_temp(&temp_path, &mut cleanup, outcome.temp_moved)?;
    verify_manifest_file(install_dir, expected)
}

fn install_manifest_symlink(install_dir: &Path, expected: &LocalModelSymlink) -> Result<()> {
    let target = target_path_with_real_parent(install_dir, &expected.path)?;
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let actual = std::fs::read_link(&target)
                .with_context(|| format!("read imported symlink {}", target.display()))?;
            if actual != Path::new(&expected.link_target) {
                bail!(
                    "existing immutable local model symlink {} points to {}, expected {}",
                    target.display(),
                    actual.display(),
                    expected.link_target
                );
            }
            Ok(())
        }
        Ok(_) => bail!(
            "existing immutable local model symlink target is not a symlink: {}",
            target.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_file_symlink(Path::new(&expected.link_target), &target).with_context(|| {
                format!(
                    "publish immutable local model symlink {} -> {}",
                    target.display(),
                    expected.link_target
                )
            })?;
            sync_directory(
                target
                    .parent()
                    .context("immutable symlink should have a parent")?,
            )
        }
        Err(error) => {
            Err(error).with_context(|| format!("stat import symlink {}", target.display()))
        }
    }
}

fn install_bytes_if_absent(install_dir: &Path, relative: &str, content: &[u8]) -> Result<()> {
    let target = target_path_with_real_parent(install_dir, relative)?;
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let actual =
                std::fs::read(&target).with_context(|| format!("read {}", target.display()))?;
            if actual != content {
                bail!(
                    "existing immutable control file differs from candidate: {}",
                    target.display()
                );
            }
            return Ok(());
        }
        Ok(_) => bail!(
            "existing immutable control path is not a regular file: {}",
            target.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("stat immutable control {}", target.display()));
        }
    }
    let (temp_path, mut temp_file) = create_publish_temp(&target)?;
    let mut cleanup = TempFileCleanup::new(temp_path.clone());
    temp_file
        .write_all(content)
        .with_context(|| format!("write immutable control temp {}", temp_path.display()))?;
    temp_file
        .sync_all()
        .with_context(|| format!("sync immutable control temp {}", temp_path.display()))?;
    drop(temp_file);
    let outcome = publish_no_clobber(&temp_path, &target).with_context(|| {
        format!(
            "publish immutable control file {} from {}",
            target.display(),
            temp_path.display()
        )
    })?;
    if outcome.target_preexisted {
        let actual = std::fs::read(&target)
            .with_context(|| format!("read raced immutable control {}", target.display()))?;
        if actual != content {
            bail!(
                "raced immutable control file differs from candidate: {}",
                target.display()
            );
        }
    }
    finish_publish_temp(&temp_path, &mut cleanup, outcome.temp_moved)?;
    Ok(())
}

struct NoClobberPublish {
    target_preexisted: bool,
    temp_moved: bool,
}

fn publish_no_clobber(temp_path: &Path, target: &Path) -> Result<NoClobberPublish> {
    match std::fs::hard_link(temp_path, target) {
        Ok(()) => Ok(NoClobberPublish {
            target_preexisted: false,
            temp_moved: false,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Ok(NoClobberPublish {
                target_preexisted: true,
                temp_moved: false,
            })
        }
        Err(hard_link_error) => match rename_no_replace(temp_path, target) {
            Ok(()) => Ok(NoClobberPublish {
                target_preexisted: false,
                temp_moved: true,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Ok(NoClobberPublish {
                    target_preexisted: true,
                    temp_moved: false,
                })
            }
            Err(rename_error) => Err(anyhow::anyhow!(
                "hard-link publish failed: {hard_link_error}; no-clobber rename fallback failed: {rename_error}"
            )),
        },
    }
}

fn finish_publish_temp(
    temp_path: &Path,
    cleanup: &mut TempFileCleanup,
    temp_moved: bool,
) -> Result<()> {
    if !temp_moved {
        std::fs::remove_file(temp_path)
            .with_context(|| format!("remove immutable publish temp {}", temp_path.display()))?;
    }
    cleanup.disarm();
    sync_directory(
        temp_path
            .parent()
            .context("immutable publish temp should have a parent")?,
    )
}

fn target_path_with_real_parent(install_dir: &Path, relative: &str) -> Result<PathBuf> {
    let canonical_install_dir = std::fs::canonicalize(install_dir)
        .with_context(|| format!("canonicalize local model install {}", install_dir.display()))?;
    let relative = checked_relative_path(relative)?;
    let parent = relative
        .parent()
        .context("local model import target must have a parent")?;
    let mut current = canonical_install_dir.clone();
    for component in parent.components() {
        let std::path::Component::Normal(component) = component else {
            bail!("local model import parent contains invalid component");
        };
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
                let canonical = std::fs::canonicalize(&current).with_context(|| {
                    format!("canonicalize local model import dir {}", current.display())
                })?;
                if canonical != current || !canonical.starts_with(&canonical_install_dir) {
                    bail!(
                        "local model import directory escapes install: {}",
                        current.display()
                    );
                }
            }
            Ok(_) => bail!(
                "local model import parent is not a real directory: {}",
                current.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = current
                    .parent()
                    .context("local model import directory should have a parent")?
                    .to_path_buf();
                match std::fs::create_dir(&current) {
                    Ok(()) => {
                        sync_directory(&parent)?;
                        sync_directory(&current)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let metadata = std::fs::symlink_metadata(&current).with_context(|| {
                            format!("stat raced import dir {}", current.display())
                        })?;
                        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
                            bail!(
                                "raced local model import parent is not a real directory: {}",
                                current.display()
                            );
                        }
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("create local model import dir {}", current.display())
                        });
                    }
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("stat local model import dir {}", current.display()));
            }
        }
    }
    Ok(canonical_install_dir.join(relative))
}

fn create_publish_temp(path: &Path) -> Result<(PathBuf, File)> {
    let parent = path
        .parent()
        .context("published local model path must have a parent")?;
    let file_name = path
        .file_name()
        .context("published local model path must have a file name")?;
    for _ in 0..32 {
        let mut name = OsString::from(".");
        name.push(file_name);
        name.push(format!(
            ".remem-tmp.{}.{}",
            std::process::id(),
            PUBLISH_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let temp_path = parent.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("create publish temp {}", temp_path.display()));
            }
        }
    }
    bail!(
        "could not create unique publish temp for {}",
        path.display()
    )
}

struct TempFileCleanup {
    path: PathBuf,
    armed: bool,
}

impl TempFileCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                crate::log::error(
                    "embedding",
                    &format!(
                        "remove failed model publish temp {}: {error}",
                        self.path.display()
                    ),
                );
            }
        }
    }
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .with_context(|| format!("open imported model directory {}", path.display()))?
        .sync_all()
        .with_context(|| format!("sync imported model directory {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(target_os = "macos")]
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both paths are valid NUL-terminated strings for the duration of the call.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // SAFETY: both paths are valid NUL-terminated strings for the duration of the call.
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE as _,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        #[link_name = "MoveFileExW"]
        fn move_file_ex_w(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }

    let existing = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let new = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both path buffers are NUL-terminated and remain alive for the call.
    if unsafe { move_file_ex_w(existing.as_ptr(), new.as_ptr(), MOVEFILE_WRITE_THROUGH) } != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "android",
    windows
)))]
fn rename_no_replace(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no-clobber rename is not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_clobber_rename_never_replaces_existing_target() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "remem-no-clobber-rename-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir(&root)?;
        let from = root.join("from");
        let to = root.join("to");
        std::fs::write(&from, b"candidate")?;
        std::fs::write(&to, b"existing")?;

        let error = rename_no_replace(&from, &to).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&from)?, b"candidate");
        assert_eq!(std::fs::read(&to)?, b"existing");
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }
}
