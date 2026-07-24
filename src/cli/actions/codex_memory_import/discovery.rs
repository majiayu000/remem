//! Safe, read-only discovery of Codex rollout-summary files (GH-852 B-005,
//! B-006, B-011). The walk never follows symlinks, rejects unknown entries
//! instead of skipping them, and performs a stable read (metadata check before
//! and after reading) so concurrent rewrites fail the batch visibly.

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

use super::redact_home;

/// Conservative proposal defaults (maintainer-adjustable, flagged in the PR
/// body per the GH-852 approval record). Real-host evidence: rollout summary
/// files observed at 3-25 KiB on codex-cli 0.145.0.
pub(super) const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub(super) const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const MAX_FILES: usize = 10_000;

#[derive(Debug)]
pub(super) enum SourceDiscovery {
    /// Source directory does not exist: explicit "no native memories" state.
    NotConfigured,
    Ready(Vec<DiscoveredFile>),
}

#[derive(Debug, Clone)]
pub(super) struct DiscoveredFile {
    /// Source-relative identifier, safe to show in diagnostics.
    pub rel_id: String,
    pub content: String,
}

pub(super) fn discover_source(source_dir: &Path) -> Result<SourceDiscovery> {
    match fs::symlink_metadata(source_dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SourceDiscovery::NotConfigured);
        }
        Err(err) => {
            bail!(
                "codex memories source {} is not readable: {err}",
                redact_home(source_dir)
            );
        }
        Ok(metadata) if !metadata.is_dir() => {
            bail!(
                "codex memories source {} exists but is not a directory",
                redact_home(source_dir)
            );
        }
        Ok(_) => {}
    }

    let entries = fs::read_dir(source_dir)
        .with_context(|| format!("read codex memories source {}", redact_home(source_dir)))?;

    let mut files = Vec::new();
    let mut total_bytes: u64 = 0;
    for entry in entries {
        let entry =
            entry.with_context(|| format!("enumerate entry under {}", redact_home(source_dir)))?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            bail!(
                "codex memories source contains a non-UTF-8 file name under {}",
                redact_home(source_dir)
            );
        };
        let metadata = fs::symlink_metadata(entry.path())
            .with_context(|| format!("stat codex memories entry {name}"))?;
        if metadata.file_type().is_symlink() {
            bail!("codex memories entry {name} is a symlink; refusing to follow it");
        }
        if metadata.is_dir() {
            bail!("codex memories source contains unexpected subdirectory {name}");
        }
        if !is_rollout_summary_filename(name) {
            bail!(
                "codex memories entry {name} does not match any supported format \
                 (supported: codex-rollout-summary/v1)"
            );
        }
        if metadata.len() > MAX_FILE_BYTES {
            bail!(
                "codex memories entry {name} is {} bytes, above the {} byte per-file limit",
                metadata.len(),
                MAX_FILE_BYTES
            );
        }
        total_bytes = total_bytes.saturating_add(metadata.len());
        if total_bytes > MAX_TOTAL_BYTES {
            bail!(
                "codex memories source exceeds the {} byte total limit",
                MAX_TOTAL_BYTES
            );
        }
        if files.len() >= MAX_FILES {
            bail!("codex memories source exceeds the {MAX_FILES} file limit");
        }

        let content = stable_read(&entry.path(), name)?;
        files.push(DiscoveredFile {
            rel_id: name.to_string(),
            content,
        });
    }

    files.sort_by(|a, b| a.rel_id.cmp(&b.rel_id));
    Ok(SourceDiscovery::Ready(files))
}

/// Read a file and verify its metadata is identical before and after the read,
/// so a concurrent rewrite fails the batch instead of importing torn content.
fn stable_read(path: &Path, name: &str) -> Result<String> {
    let before = fs::symlink_metadata(path)
        .with_context(|| format!("stat codex memories entry {name} before read"))?;
    let bytes = fs::read(path).with_context(|| format!("read codex memories entry {name}"))?;
    let after = fs::symlink_metadata(path)
        .with_context(|| format!("stat codex memories entry {name} after read"))?;
    let same = before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && before.len() == bytes.len() as u64;
    if !same {
        bail!("codex memories entry {name} changed while being read; retry the import");
    }
    String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("codex memories entry {name} is not valid UTF-8"))
}

/// Delegates to the shared install-path fingerprint (single source with the
/// doctor check).
pub(super) fn is_rollout_summary_filename(name: &str) -> bool {
    crate::install::is_codex_rollout_summary_filename(name)
}
