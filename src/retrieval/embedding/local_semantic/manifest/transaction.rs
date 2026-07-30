use std::fs::File;
#[cfg(feature = "local-onnx")]
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::super::{
    checked_relative_path, is_sha256_hex, LocalEmbeddingPreset, LocalModelManifest, MANIFEST_FILE,
};
use super::locks::canonical_real_install_dir;
use super::sha256_bytes;

mod atomic;

use atomic::{atomic_replace_control_file, remove_candidate_control_file, sync_parent};
#[cfg(feature = "local-onnx")]
use atomic::{create_transaction_temp, replace_file, TempFileCleanup};

const ACTIVATION_JOURNAL_FILE: &str = ".remem-model-activation.json";
const ACTIVATION_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivationJournal {
    schema_version: u32,
    phase: ActivationPhase,
    preset: String,
    ref_relative: String,
    candidate_ref: Vec<u8>,
    previous_ref: Option<ControlFileSnapshot>,
    previous_manifest: Option<ControlFileSnapshot>,
    previous_manifest_sha256: Option<String>,
    candidate_manifest: Vec<u8>,
    candidate_manifest_sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ActivationPhase {
    Prepared,
    Committing,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlFileSnapshot {
    content: Vec<u8>,
    permissions: StoredPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredPermissions {
    readonly: bool,
    #[serde(default)]
    unix_mode: Option<u32>,
}

#[cfg(feature = "local-onnx")]
pub(in crate::retrieval::embedding::local_semantic) struct ActiveRevisionTransaction {
    install_dir: PathBuf,
    armed: bool,
}

#[cfg(feature = "local-onnx")]
impl ActiveRevisionTransaction {
    pub(in crate::retrieval::embedding::local_semantic) fn begin(
        install_dir: &Path,
        preset: LocalEmbeddingPreset,
        revision: &str,
        candidate_manifest: &LocalModelManifest,
    ) -> Result<Self> {
        let install_dir = canonical_real_install_dir(install_dir)?;
        recover_pending_activation(&install_dir)?;
        let candidate_ref = validated_revision(revision)?.into_bytes();
        let ref_relative = format!("{}/refs/main", preset.cache_repo_dir());
        let ref_path = control_path(&install_dir, &ref_relative)?;
        let previous_ref = capture_control_file(&ref_path)?;
        let previous_manifest = capture_manifest_file(&install_dir)?;
        let previous_manifest_sha256 = previous_manifest
            .as_ref()
            .map(|manifest| sha256_bytes(&manifest.content));
        let candidate_manifest = serde_json::to_vec_pretty(candidate_manifest)?;
        let candidate_manifest_sha256 = sha256_bytes(&candidate_manifest);
        let journal = ActivationJournal {
            schema_version: ACTIVATION_JOURNAL_SCHEMA_VERSION,
            phase: ActivationPhase::Prepared,
            preset: preset.label().to_string(),
            ref_relative,
            candidate_ref: candidate_ref.clone(),
            previous_ref,
            previous_manifest,
            previous_manifest_sha256,
            candidate_manifest,
            candidate_manifest_sha256,
        };
        write_journal(&install_dir, &journal)?;
        if let Err(error) =
            replace_control_from_snapshot(&ref_path, &candidate_ref, journal.previous_ref.as_ref())
        {
            return resolve_begin_error(&install_dir, error);
        }
        Ok(Self {
            install_dir,
            armed: true,
        })
    }

    pub(in crate::retrieval::embedding::local_semantic) fn resolve(&mut self) -> Result<()> {
        if !self.armed {
            return Ok(());
        }
        recover_pending_activation(&self.install_dir)?;
        self.armed = false;
        Ok(())
    }

    pub(in crate::retrieval::embedding::local_semantic) fn mark_committing(
        &mut self,
    ) -> Result<()> {
        if !self.armed {
            bail!("local model activation transaction is not active");
        }
        let mut journal = read_journal(&self.install_dir)?
            .context("local model activation journal disappeared before commit")?;
        validate_journal(&journal)?;
        if journal.phase != ActivationPhase::Prepared {
            bail!("local model activation journal entered unexpected phase before commit");
        }
        if manifest_sha256(&self.install_dir)? != journal.previous_manifest_sha256 {
            bail!("local model manifest changed before activation commit");
        }
        match capture_control_file(&control_path(&self.install_dir, &journal.ref_relative)?)? {
            Some(current) if current.content == journal.candidate_ref => {}
            _ => bail!("local model active revision changed before activation commit"),
        }
        journal.phase = ActivationPhase::Committing;
        replace_journal(&self.install_dir, &journal)
    }

    pub(in crate::retrieval::embedding::local_semantic) fn commit(mut self) -> Result<()> {
        let journal = read_journal(&self.install_dir)?
            .context("local model activation journal disappeared before commit")?;
        validate_journal(&journal)?;
        if journal.phase != ActivationPhase::Committing
            || manifest_sha256(&self.install_dir)?.as_deref()
                != Some(&journal.candidate_manifest_sha256)
        {
            bail!("local model activation did not reach a committed manifest state");
        }
        match capture_control_file(&control_path(&self.install_dir, &journal.ref_relative)?)? {
            Some(current) if current.content == journal.candidate_ref => {}
            _ => bail!("local model activation did not reach a committed revision state"),
        }
        remove_journal(&self.install_dir)?;
        self.armed = false;
        Ok(())
    }
}

#[cfg(feature = "local-onnx")]
impl Drop for ActiveRevisionTransaction {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(error) = self.resolve() {
            crate::log::error(
                "embedding",
                &format!(
                    "recover interrupted local model activation {}: {error:#}",
                    self.install_dir.display()
                ),
            );
        }
    }
}

pub(in crate::retrieval::embedding::local_semantic) fn activation_pending(
    install_dir: &Path,
) -> Result<bool> {
    let install_dir = canonical_real_install_dir(install_dir)?;
    let path = install_dir.join(ACTIVATION_JOURNAL_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => bail!(
            "local model activation journal is not a real file: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("stat activation journal {}", path.display()))
        }
    }
}

pub(in crate::retrieval::embedding::local_semantic) fn recover_pending_activation(
    install_dir: &Path,
) -> Result<()> {
    let install_dir = canonical_real_install_dir(install_dir)?;
    let Some(journal) = read_journal(&install_dir)? else {
        return Ok(());
    };
    validate_journal(&journal)?;
    let preset = LocalEmbeddingPreset::parse(&journal.preset)?;
    let expected_ref_relative = format!("{}/refs/main", preset.cache_repo_dir());
    if journal.ref_relative != expected_ref_relative {
        bail!(
            "activation journal ref {} does not match preset {}",
            journal.ref_relative,
            preset.label()
        );
    }
    let ref_path = control_path(&install_dir, &journal.ref_relative)?;
    match journal.phase {
        ActivationPhase::Prepared => {
            if manifest_sha256(&install_dir)? != journal.previous_manifest_sha256 {
                bail!("cannot roll back prepared local model activation because manifest changed");
            }
            restore_previous_ref(&ref_path, &journal)?;
        }
        ActivationPhase::Committing => {
            ensure_candidate_ref(&ref_path, &journal)?;
            if let Err(error) = verify_journal_candidate(&install_dir, preset, &journal) {
                restore_previous_ref(&ref_path, &journal)?;
                restore_previous_manifest(&install_dir, &journal)?;
                crate::log::error(
                    "embedding",
                    &format!(
                        "rolled back interrupted local model activation after candidate verification failed: {error:#}"
                    ),
                );
            } else {
                finish_candidate_manifest(&install_dir, &journal)?;
            }
        }
    }
    remove_journal(&install_dir)
}

#[cfg(feature = "local-onnx")]
fn resolve_begin_error<T>(install_dir: &Path, error: anyhow::Error) -> Result<T> {
    match recover_pending_activation(install_dir) {
        Ok(()) => Err(error),
        Err(recovery_error) => Err(anyhow::anyhow!(
            "{error:#}; additionally failed to recover activation transaction: {recovery_error:#}"
        )),
    }
}

fn validate_journal(journal: &ActivationJournal) -> Result<()> {
    if journal.schema_version != ACTIVATION_JOURNAL_SCHEMA_VERSION {
        bail!(
            "unsupported local model activation journal schema {}",
            journal.schema_version
        );
    }
    let candidate = std::str::from_utf8(&journal.candidate_ref)
        .context("activation journal candidate revision is not UTF-8")?;
    validated_revision(candidate)?;
    if !is_sha256_hex(&journal.candidate_manifest_sha256)
        || journal
            .previous_manifest_sha256
            .as_deref()
            .is_some_and(|sha256| !is_sha256_hex(sha256))
    {
        bail!("activation journal contains invalid manifest SHA-256");
    }
    if sha256_bytes(&journal.candidate_manifest) != journal.candidate_manifest_sha256 {
        bail!("activation journal candidate manifest checksum is invalid");
    }
    let _: LocalModelManifest = serde_json::from_slice(&journal.candidate_manifest)
        .context("activation journal candidate manifest is invalid")?;
    match (
        journal.previous_manifest.as_ref(),
        journal.previous_manifest_sha256.as_deref(),
    ) {
        (Some(previous), Some(expected)) if sha256_bytes(&previous.content) == expected => {}
        (None, None) => {}
        _ => bail!("activation journal previous manifest snapshot is inconsistent"),
    }
    if let Some(previous) = &journal.previous_ref {
        let previous = std::str::from_utf8(&previous.content)
            .context("activation journal previous revision is not UTF-8")?;
        validated_revision(previous)?;
    }
    Ok(())
}

fn verify_journal_candidate(
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
    journal: &ActivationJournal,
) -> Result<()> {
    let manifest: LocalModelManifest = serde_json::from_slice(&journal.candidate_manifest)
        .context("parse activation journal candidate manifest")?;
    let verified = super::verify_unpublished_manifest(install_dir, &manifest, Some(preset))
        .context("verify interrupted activation candidate")?;
    if verified != super::model_content_sha256(&manifest)? {
        bail!("interrupted activation candidate identity changed");
    }
    Ok(())
}

fn finish_candidate_manifest(install_dir: &Path, journal: &ActivationJournal) -> Result<()> {
    let current = manifest_sha256(install_dir)?;
    if current.as_deref() == Some(&journal.candidate_manifest_sha256) {
        return Ok(());
    }
    if current != journal.previous_manifest_sha256 {
        bail!(
            "cannot finish local model activation because manifest matches neither previous nor candidate"
        );
    }
    let manifest: LocalModelManifest = serde_json::from_slice(&journal.candidate_manifest)
        .context("parse committed local model candidate manifest")?;
    super::write_manifest(install_dir, &manifest)
        .context("finish interrupted local model manifest publish")?;
    let actual = manifest_sha256(install_dir)?;
    if actual.as_deref() != Some(&journal.candidate_manifest_sha256) {
        bail!("finished local model manifest does not match activation journal");
    }
    Ok(())
}

fn ensure_candidate_ref(path: &Path, journal: &ActivationJournal) -> Result<()> {
    match capture_control_file(path)? {
        Some(current) if current.content == journal.candidate_ref => Ok(()),
        Some(current)
            if journal
                .previous_ref
                .as_ref()
                .is_some_and(|previous| current.content == previous.content) =>
        {
            atomic_replace_control_file(
                path,
                &journal.candidate_ref,
                Some(&current.content),
                Some(&current.permissions),
            )
        }
        None if journal.previous_ref.is_none() => {
            atomic_replace_control_file(path, &journal.candidate_ref, None, None)
        }
        _ => bail!(
            "active revision ref changed outside recoverable activation transaction: {}",
            path.display()
        ),
    }
}

fn restore_previous_ref(path: &Path, journal: &ActivationJournal) -> Result<()> {
    let current = capture_control_file(path)?;
    match (&journal.previous_ref, current) {
        (Some(previous), Some(current)) if current.content == previous.content => Ok(()),
        (Some(previous), Some(current)) if current.content == journal.candidate_ref => {
            atomic_replace_control_file(
                path,
                &previous.content,
                Some(&current.content),
                Some(&previous.permissions),
            )
        }
        (None, Some(current)) if current.content == journal.candidate_ref => {
            remove_candidate_control_file(path, &current.content)
        }
        (None, None) => Ok(()),
        _ => bail!(
            "active revision ref changed outside recoverable activation transaction: {}",
            path.display()
        ),
    }
}

fn restore_previous_manifest(install_dir: &Path, journal: &ActivationJournal) -> Result<()> {
    let path = install_dir.join(MANIFEST_FILE);
    let current = capture_control_file(&path)?;
    match (&journal.previous_manifest, current) {
        (Some(previous), Some(current)) if current.content == previous.content => Ok(()),
        (Some(previous), Some(current))
            if sha256_bytes(&current.content) == journal.candidate_manifest_sha256 =>
        {
            atomic_replace_control_file(
                &path,
                &previous.content,
                Some(&current.content),
                Some(&previous.permissions),
            )
        }
        (Some(previous), None) => {
            atomic_replace_control_file(&path, &previous.content, None, Some(&previous.permissions))
        }
        (None, Some(current))
            if sha256_bytes(&current.content) == journal.candidate_manifest_sha256 =>
        {
            remove_candidate_control_file(&path, &current.content)
        }
        (None, None) => Ok(()),
        _ => bail!(
            "manifest changed outside recoverable activation transaction: {}",
            path.display()
        ),
    }
}

#[cfg(feature = "local-onnx")]
fn replace_control_from_snapshot(
    path: &Path,
    content: &[u8],
    previous: Option<&ControlFileSnapshot>,
) -> Result<()> {
    atomic_replace_control_file(
        path,
        content,
        previous.map(|previous| previous.content.as_slice()),
        previous.map(|previous| &previous.permissions),
    )
}

fn capture_control_file(path: &Path) -> Result<Option<ControlFileSnapshot>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let content = std::fs::read(path)
                .with_context(|| format!("read control file {}", path.display()))?;
            Ok(Some(ControlFileSnapshot {
                content,
                permissions: StoredPermissions::capture(&metadata),
            }))
        }
        Ok(_) => bail!(
            "refusing to replace non-regular active revision ref {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("stat control file {}", path.display())),
    }
}

#[cfg(feature = "local-onnx")]
fn capture_manifest_file(install_dir: &Path) -> Result<Option<ControlFileSnapshot>> {
    capture_control_file(&install_dir.join(MANIFEST_FILE))
        .context("capture previous local model manifest")
}

impl StoredPermissions {
    fn capture(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            Self {
                readonly: metadata.permissions().readonly(),
                unix_mode: Some(metadata.permissions().mode()),
            }
        }
        #[cfg(not(unix))]
        {
            Self {
                readonly: metadata.permissions().readonly(),
                unix_mode: None,
            }
        }
    }

    fn apply(&self, file: &File) -> Result<()> {
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::Permissions::from_mode(
                self.unix_mode
                    .context("activation journal is missing Unix control permissions")?,
            )
        };
        #[cfg(not(unix))]
        let permissions = {
            let mut permissions = file.metadata()?.permissions();
            permissions.set_readonly(self.readonly);
            permissions
        };
        file.set_permissions(permissions)
            .context("restore active revision ref permissions")
    }
}

fn manifest_sha256(install_dir: &Path) -> Result<Option<String>> {
    let path = install_dir.join(MANIFEST_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            let content = std::fs::read(&path)
                .with_context(|| format!("read manifest state {}", path.display()))?;
            Ok(Some(sha256_bytes(&content)))
        }
        Ok(_) => bail!(
            "local model manifest is not a real file: {}",
            path.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("stat manifest state {}", path.display())),
    }
}

fn control_path(install_dir: &Path, relative: &str) -> Result<PathBuf> {
    let relative = checked_relative_path(relative)?;
    let path = install_dir.join(&relative);
    let parent = path
        .parent()
        .context("active revision ref should have a parent")?;
    let metadata = std::fs::symlink_metadata(parent)
        .with_context(|| format!("stat active revision parent {}", parent.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        bail!(
            "active revision parent is not a real directory: {}",
            parent.display()
        );
    }
    let canonical_parent = std::fs::canonicalize(parent)
        .with_context(|| format!("canonicalize active revision parent {}", parent.display()))?;
    if canonical_parent != parent || !canonical_parent.starts_with(install_dir) {
        bail!(
            "active revision parent escapes local model install: {}",
            canonical_parent.display()
        );
    }
    Ok(path)
}

fn validated_revision(raw: &str) -> Result<String> {
    let revision = raw.trim();
    if !matches!(revision.len(), 40 | 64) || !revision.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("invalid activation journal Hugging Face revision");
    }
    Ok(revision.to_string())
}

#[cfg(feature = "local-onnx")]
fn write_journal(install_dir: &Path, journal: &ActivationJournal) -> Result<()> {
    let path = install_dir.join(ACTIVATION_JOURNAL_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => bail!(
            "local model activation journal already exists: {}",
            path.display()
        ),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("stat activation journal {}", path.display()));
        }
    }
    let (temp_path, mut temp_file) = create_transaction_temp(&path)?;
    let mut cleanup = TempFileCleanup::new(temp_path.clone());
    let content = serde_json::to_vec_pretty(journal)?;
    temp_file
        .write_all(&content)
        .with_context(|| format!("write activation journal temp {}", temp_path.display()))?;
    temp_file
        .sync_all()
        .with_context(|| format!("sync activation journal temp {}", temp_path.display()))?;
    drop(temp_file);
    std::fs::rename(&temp_path, &path)
        .with_context(|| format!("publish activation journal {}", path.display()))?;
    cleanup.disarm();
    sync_parent(install_dir)
}

#[cfg(feature = "local-onnx")]
fn replace_journal(install_dir: &Path, journal: &ActivationJournal) -> Result<()> {
    let path = install_dir.join(ACTIVATION_JOURNAL_FILE);
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("stat activation journal {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "local model activation journal is not a real file: {}",
            path.display()
        );
    }
    let (temp_path, mut temp_file) = create_transaction_temp(&path)?;
    let mut cleanup = TempFileCleanup::new(temp_path.clone());
    let content = serde_json::to_vec_pretty(journal)?;
    temp_file
        .write_all(&content)
        .with_context(|| format!("write activation journal update {}", temp_path.display()))?;
    temp_file
        .sync_all()
        .with_context(|| format!("sync activation journal update {}", temp_path.display()))?;
    drop(temp_file);
    let current_metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("recheck activation journal {}", path.display()))?;
    if current_metadata.file_type().is_symlink() || !current_metadata.file_type().is_file() {
        bail!(
            "local model activation journal changed type before update: {}",
            path.display()
        );
    }
    replace_file(&temp_path, &path)?;
    cleanup.disarm();
    sync_parent(install_dir)
}

fn read_journal(install_dir: &Path) -> Result<Option<ActivationJournal>> {
    let path = install_dir.join(ACTIVATION_JOURNAL_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(metadata) if metadata.file_type().is_file() => {
            let content = std::fs::read(&path)
                .with_context(|| format!("read activation journal {}", path.display()))?;
            Ok(Some(serde_json::from_slice(&content).with_context(
                || format!("parse activation journal {}", path.display()),
            )?))
        }
        Ok(_) => bail!(
            "local model activation journal is not a real file: {}",
            path.display()
        ),
        Err(error) => {
            Err(error).with_context(|| format!("stat activation journal {}", path.display()))
        }
    }
}

fn remove_journal(install_dir: &Path) -> Result<()> {
    let path = install_dir.join(ACTIVATION_JOURNAL_FILE);
    let metadata = std::fs::symlink_metadata(&path)
        .with_context(|| format!("stat activation journal {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "refusing to remove non-file activation journal {}",
            path.display()
        );
    }
    std::fs::remove_file(&path)
        .with_context(|| format!("remove activation journal {}", path.display()))?;
    sync_parent(install_dir)
}
