//! Secure staged writer and two-file + runtime-config staged apply with
//! compensating rollback (GH-824 B-010..B-013).
//!
//! This is deliberately not `crate::atomic_file::write_atomic`: Cursor MCP
//! files can carry secrets in foreign `env` maps, so the fresh temp file must
//! be owner-only before the first byte is written, and the apply path needs
//! final-comparison / read-back / compensating-rollback semantics that the
//! generic writer does not provide.
//!
//! Honest boundary (B-012): compare-then-rename is not CAS. A non-cooperating
//! writer that lands between the final comparison and the rename can be
//! overwritten by the planned rename and, when read-back only observes the
//! planned bytes, cannot be detected afterwards. That residual
//! user-data-loss window is documented, not claimed away.

use anyhow::{bail, Context, Result};
use std::io::Write;
use std::path::Path;

use super::plan::{CursorConfigPlan, FileAction, FilePlan};

#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
static FAIL_RENAME_FOR_PATHS: Mutex<Vec<std::path::PathBuf>> = Mutex::new(Vec::new());

#[cfg(test)]
pub(crate) fn fail_next_rename_for_path_for_test(path: &Path) {
    FAIL_RENAME_FOR_PATHS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(path.to_path_buf());
}

#[cfg(test)]
pub(crate) fn clear_failpoints_for_test() {
    FAIL_RENAME_FOR_PATHS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
}

fn rename_failpoint(path: &Path) -> Result<()> {
    #[cfg(test)]
    {
        let mut paths = FAIL_RENAME_FOR_PATHS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(position) = paths.iter().position(|candidate| candidate == path) {
            paths.remove(position);
            bail!(
                "injected cursor staged rename failure for {}",
                path.display()
            );
        }
    }
    let _ = path;
    Ok(())
}

fn read_current(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
    }
}

/// Writes `bytes` to `path` through a fresh owner-only temp file. The temp is
/// created with `create_new` and mode `0o600` before the first byte is
/// written (B-013); non-Unix platforms fail closed (the Cursor host is
/// unsupported there anyway).
fn write_secure(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (path, bytes);
        bail!("cursor secure staged writer is only approved for Unix platforms (code=platform_unsupported)");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let parent = path
            .parent()
            .with_context(|| format!("{} has no parent directory", path.display()))?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create directory {}", parent.display()))?;
        let temp_path = parent.join(format!(
            ".remem-cursor-{}-{}.tmp",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp_path)
                .with_context(|| format!("create secure temp for {}", path.display()))?;
            // `mode(0o600)` is subject to the process umask; normalize to
            // exactly owner-only before any byte is written.
            file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))
                .with_context(|| {
                    format!("set owner-only permissions on temp for {}", path.display())
                })?;
            file.write_all(bytes)
                .with_context(|| format!("write staged bytes for {}", path.display()))?;
            file.sync_all()
                .with_context(|| format!("sync staged bytes for {}", path.display()))?;
            drop(file);
            rename_failpoint(path)?;
            std::fs::rename(&temp_path, path)
                .with_context(|| format!("rename staged file into {}", path.display()))?;
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }
        result
    }
}

/// Final comparison immediately before a replace/restore (B-012): the current
/// bytes must equal `expected`; an observable external edit aborts and is
/// preserved.
fn final_compare(path: &Path, expected: Option<&[u8]>) -> Result<()> {
    let current = read_current(path)?;
    if current.as_deref() != expected {
        bail!(
            "{} changed since it was validated; aborting and preserving the external version (code=concurrent_edit)",
            path.display()
        );
    }
    Ok(())
}

struct CommittedTarget<'a> {
    plan: &'a FilePlan,
}

/// Applies every mutating target of the plan in fixed order (hooks.json,
/// mcp.json, runtime config) with per-target final comparison, secure
/// replace, and read-back. Later failures trigger reverse compensating
/// rollback; rollback failures surface `partial_state` with per-path status
/// and doctor guidance (B-010/B-011).
pub(crate) fn apply_plan(plan: &CursorConfigPlan) -> Result<()> {
    let targets: Vec<&FilePlan> = [&plan.hooks, &plan.mcp, &plan.runtime_config]
        .into_iter()
        .filter(|target| target.action != FileAction::NoOp && target.new_bytes.is_some())
        .collect();

    let mut committed: Vec<CommittedTarget> = Vec::new();
    for target in &targets {
        let apply_result = apply_single(target);
        match apply_result {
            Ok(()) => committed.push(CommittedTarget { plan: target }),
            Err(error) => {
                return Err(rollback_committed(&committed, error));
            }
        }
    }
    Ok(())
}

fn apply_single(target: &FilePlan) -> Result<()> {
    let path = &target.snapshot.path;
    let planned = target
        .new_bytes
        .as_deref()
        .expect("apply_single only receives mutating targets");
    final_compare(path, target.snapshot.bytes.as_deref())?;
    write_secure(path, planned)?;
    let read_back = read_current(path)?;
    if read_back.as_deref() != Some(planned) {
        bail!(
            "{} drifted immediately after replace (read-back mismatch); reporting partial_state (code=read_back_drift)",
            path.display()
        );
    }
    Ok(())
}

fn rollback_committed(committed: &[CommittedTarget<'_>], original: anyhow::Error) -> anyhow::Error {
    let mut failures: Vec<String> = Vec::new();
    for target in committed.iter().rev() {
        if let Err(error) = restore_snapshot(target.plan) {
            failures.push(format!(
                "{}: {error:#}",
                target.plan.snapshot.path.display()
            ));
        }
    }
    if failures.is_empty() {
        original.context(
            "cursor staged apply failed; all previously committed targets were restored via compensating rollback",
        )
    } else {
        original.context(format!(
            "partial_state: cursor compensating rollback failed for [{}]; run `remem doctor` and repair the listed paths before retrying install/uninstall",
            failures.join("; ")
        ))
    }
}

/// Restores one committed target back to its plan snapshot. The restore only
/// proceeds when the current content still equals the bytes this transaction
/// wrote; an externally observed version is preserved (B-012).
fn restore_snapshot(plan: &FilePlan) -> Result<()> {
    let path = &plan.snapshot.path;
    let planned = plan
        .new_bytes
        .as_deref()
        .expect("only mutating targets are committed");
    let current = read_current(path)?;
    if current.as_deref() != Some(planned) {
        bail!(
            "current content no longer matches this transaction's bytes; preserving the external version (code=concurrent_edit)"
        );
    }
    match plan.snapshot.bytes.as_deref() {
        Some(previous) => write_secure(path, previous)?,
        None => {
            std::fs::remove_file(path)
                .with_context(|| format!("remove {} during rollback", path.display()))?;
        }
    }
    let after = read_current(path)?;
    if after.as_deref() != plan.snapshot.bytes.as_deref() {
        bail!(
            "rollback read-back does not match the original snapshot (code=rollback_verify_failed)"
        );
    }
    Ok(())
}
