//! `remem ingest-sessions` — batch, incremental, idempotent ingestion of
//! Claude Code / Codex session transcripts into `raw_messages` (issue #722).
//!
//! Discovery walks each scan root for `*.jsonl` files (skipping `subagents/`
//! directories), a per-file cursor in `ingest_cursors` skips files whose
//! mtime and size are unchanged, and each hit is drained through the existing
//! `drain_transcript` path so the `raw_messages` UNIQUE constraint dedupes
//! against the Stop-hook ingestion running concurrently.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::memory::raw_archive::{self, TranscriptDrainOptions, SOURCE_ROOT_LOCAL};

/// A file whose mtime is within this many seconds of now is treated as an
/// actively-appended session: a JSON parse failure on its last line is a
/// partial tail, not a file failure, and the cursor does not advance.
const ACTIVE_TAIL_WINDOW_SECS: i64 = 60;

/// One scan root: a label recorded as `raw_messages.source_root` plus the
/// directory to walk.
#[derive(Debug, Clone)]
pub struct ScanRoot {
    pub label: String,
    pub path: PathBuf,
    /// Default local roots are optional because many users only have one host
    /// installed. User-supplied `--root label=path` entries are required and
    /// must not fail silently.
    pub required: bool,
}

impl ScanRoot {
    /// Parse a `--root label=path` argument.
    pub fn parse(spec: &str) -> Result<Self> {
        let Some((label, path)) = spec.split_once('=') else {
            bail!("invalid --root {spec:?}: expected label=path");
        };
        let label = label.trim();
        let path = path.trim();
        if label.is_empty() || path.is_empty() {
            bail!("invalid --root {spec:?}: label and path must be non-empty");
        }
        Ok(Self {
            label: label.to_string(),
            path: PathBuf::from(shellexpand_home(path)),
            required: true,
        })
    }
}

fn shellexpand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    path.to_string()
}

/// Default local scan roots: `~/.claude/projects` and `~/.codex/sessions`.
/// Both are labeled `local` to match the hook-path `source_root` default.
pub fn default_scan_roots() -> Vec<ScanRoot> {
    let Some(home) = dirs::home_dir() else {
        crate::log::warn("ingest-sessions", "home directory unavailable");
        return Vec::new();
    };
    vec![
        ScanRoot {
            label: SOURCE_ROOT_LOCAL.to_string(),
            path: home.join(".claude").join("projects"),
            required: false,
        },
        ScanRoot {
            label: SOURCE_ROOT_LOCAL.to_string(),
            path: home.join(".codex").join("sessions"),
            required: false,
        },
    ]
}

/// Machine-readable batch summary (product invariant 6).
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct IngestSummary {
    pub scanned: usize,
    pub skipped: usize,
    pub ingested_messages: usize,
    pub failed_files: usize,
    pub partial_files: usize,
}

impl IngestSummary {
    pub fn exit_code(&self) -> i32 {
        if self.failed_files > 0 {
            1
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IngestOptions {
    /// Skip files whose mtime is older than this lower bound (backfill bound;
    /// window semantics on message timestamps belong to the query side).
    pub since_epoch: Option<i64>,
}

/// Run one batch ingestion pass over the given scan roots (callers build the
/// list from `default_scan_roots()` plus any `--root label=path` extras).
pub fn run_ingest_sessions(
    conn: &Connection,
    roots: &[ScanRoot],
    options: &IngestOptions,
) -> Result<IngestSummary> {
    let mut summary = IngestSummary::default();
    let now = chrono::Utc::now().timestamp();
    let mut project_cache = BTreeMap::new();

    let mut discovered = Vec::new();
    for root in roots {
        let (files, discovery_failures) = discover_transcript_files(root);
        for failure in discovery_failures {
            summary.failed_files += 1;
            crate::log::error("ingest-sessions", &failure);
        }
        for file in files {
            summary.scanned += 1;
            let plan = match super::session_identity::probe_with_project_cache(
                &root.label,
                &root.path,
                &file,
                None,
                &mut project_cache,
            ) {
                Ok(plan) => plan,
                Err(error) => {
                    summary.failed_files += 1;
                    crate::log::error(
                        "ingest-sessions",
                        &format!("identity probe {} failed: {error}", file.display()),
                    );
                    continue;
                }
            };
            let mtime_epoch = plan.observed_mtime_ns / 1_000_000_000;
            let phase_b_eligible = !options.since_epoch.is_some_and(|since| mtime_epoch < since);
            discovered.push((root.clone(), plan, phase_b_eligible));
        }
    }
    if summary.failed_files > 0 {
        crate::log::error(
            "ingest-sessions",
            "Phase A discovery/probe was incomplete; Phase B mutation is blocked",
        );
        return Ok(summary);
    }
    conn.execute_batch("SAVEPOINT gh871_identity_phase_a")?;
    let phase_a =
        (|| -> Result<Vec<(ScanRoot, super::session_identity::TranscriptPlan, i64, bool)>> {
            let mut prepared = Vec::with_capacity(discovered.len());
            let mut groups = BTreeSet::new();
            for (root, plan, phase_b_eligible) in discovered {
                let identity_id = super::session_identity::upsert_claim(conn, &plan, now)?;
                groups.insert((plan.source_root.clone(), plan.fallback_session_id.clone()));
                prepared.push((root, plan, identity_id, phase_b_eligible));
            }
            for (source_root, fallback_session_id) in groups {
                super::session_identity::resolve_fallback_group(
                    conn,
                    &source_root,
                    &fallback_session_id,
                )?;
            }
            Ok(prepared)
        })();
    let prepared = match phase_a {
        Ok(prepared) => {
            conn.execute_batch("RELEASE gh871_identity_phase_a")?;
            prepared
        }
        Err(error) => {
            conn.execute_batch(
                "ROLLBACK TO gh871_identity_phase_a; RELEASE gh871_identity_phase_a",
            )?;
            return Err(error.context("persist complete transcript identity claim set"));
        }
    };

    let mut prepared_groups = BTreeMap::new();
    for prepared_file in prepared {
        let key = (
            prepared_file.1.source_root.clone(),
            prepared_file.1.fallback_session_id.clone(),
        );
        prepared_groups
            .entry(key)
            .or_insert_with(Vec::new)
            .push(prepared_file);
    }
    for ((source_root, fallback_session_id), group) in prepared_groups {
        conn.execute_batch("SAVEPOINT gh871_identity_phase_b_group")?;
        let ingested_before = summary.ingested_messages;
        let partial_before = summary.partial_files;
        let mut identity_conflict = false;
        for (root, plan, identity_id, phase_b_eligible) in &group {
            if !phase_b_eligible {
                let indexed = super::session_identity::index_events(
                    &plan.transcript_path,
                    u64::try_from(plan.observed_size_bytes).unwrap_or(u64::MAX),
                )
                .and_then(|index| {
                    super::session_identity::record_since_skipped_event_index(
                        conn,
                        *identity_id,
                        index,
                        now,
                    )
                });
                match indexed {
                    Ok(()) => summary.skipped += 1,
                    Err(error) => {
                        summary.failed_files += 1;
                        crate::log::error(
                            "ingest-sessions",
                            &format!("index skipped {} failed: {error}", plan.path.display()),
                        );
                    }
                }
                continue;
            }
            conn.execute_batch("SAVEPOINT gh871_identity_phase_b_file")?;
            let inserted_before = summary.ingested_messages;
            let result = ingest_prepared_file(conn, root, plan, *identity_id, now, &mut summary);
            match result {
                PreparedFileResult::Commit => {
                    conn.execute_batch("RELEASE gh871_identity_phase_b_file")?;
                }
                PreparedFileResult::Rollback {
                    identity_conflict: file_identity_conflict,
                } => {
                    conn.execute_batch(
                        "ROLLBACK TO gh871_identity_phase_b_file;
                         RELEASE gh871_identity_phase_b_file",
                    )?;
                    summary.ingested_messages = inserted_before;
                    if file_identity_conflict {
                        identity_conflict = true;
                        break;
                    }
                }
            }
        }
        if identity_conflict {
            conn.execute_batch(
                "ROLLBACK TO gh871_identity_phase_b_group;
                 RELEASE gh871_identity_phase_b_group",
            )?;
            summary.ingested_messages = ingested_before;
            summary.partial_files = partial_before;
            super::session_identity::mark_fallback_group_conflict(
                conn,
                &source_root,
                &fallback_session_id,
                "stable_occurrence_mismatch",
            )?;
        } else {
            conn.execute_batch("RELEASE gh871_identity_phase_b_group")?;
        }
    }

    crate::log::info(
        "ingest-sessions",
        &format!(
            "batch done scanned={} skipped={} ingested_messages={} failed_files={} partial_files={}",
            summary.scanned,
            summary.skipped,
            summary.ingested_messages,
            summary.failed_files,
            summary.partial_files
        ),
    );
    Ok(summary)
}

pub(crate) fn discover_transcript_files(root: &ScanRoot) -> (Vec<PathBuf>, Vec<String>) {
    if !root.path.is_dir() {
        let failures = if root.required {
            vec![format!(
                "required scan root {}={} is missing or not a directory",
                root.label,
                root.path.display()
            )]
        } else {
            Vec::new()
        };
        return (Vec::new(), failures);
    }
    let mut files = Vec::new();
    let mut failures = Vec::new();
    collect_jsonl_files(&root.path, &mut files, &mut failures);
    files.sort();
    (files, failures)
}

/// Recursively collect `*.jsonl` files, excluding `subagents/` directories.
fn collect_jsonl_files(dir: &Path, out: &mut Vec<PathBuf>, failures: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            failures.push(format!("read scan dir {} failed: {}", dir.display(), error));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!(
                    "read scan dir entry in {} failed: {}",
                    dir.display(),
                    error
                ));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                failures.push(format!("stat {} failed: {}", path.display(), error));
                continue;
            }
        };
        if file_type.is_dir() {
            if entry.file_name() == "subagents" {
                continue;
            }
            collect_jsonl_files(&path, out, failures);
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
            out.push(path);
        }
    }
}

enum PreparedFileResult {
    Commit,
    Rollback { identity_conflict: bool },
}

fn ingest_prepared_file(
    conn: &Connection,
    root: &ScanRoot,
    plan: &super::session_identity::TranscriptPlan,
    identity_id: i64,
    now: i64,
    summary: &mut IngestSummary,
) -> PreparedFileResult {
    let identity = match super::session_identity::load(conn, identity_id) {
        Ok(identity) => identity,
        Err(error) => {
            summary.failed_files += 1;
            crate::log::error(
                "ingest-sessions",
                &format!("load identity {} failed: {error}", plan.path.display()),
            );
            return PreparedFileResult::Commit;
        }
    };
    if identity.status == "conflict" {
        summary.failed_files += 1;
        crate::log::error(
            "ingest-sessions",
            &format!(
                "identity conflict for transcript {}; raw rows remain unchanged",
                plan.path.display()
            ),
        );
        return PreparedFileResult::Commit;
    }
    let mtime_epoch = plan.observed_mtime_ns / 1_000_000_000;
    let size_bytes = plan.observed_size_bytes;
    match cursor_unchanged(conn, root, &plan.path, mtime_epoch, size_bytes) {
        Ok(true) if identity.contract_version >= 1 => {
            summary.skipped += 1;
            return PreparedFileResult::Commit;
        }
        Ok(true) | Ok(false) => {}
        Err(error) => {
            summary.failed_files += 1;
            crate::log::error(
                "ingest-sessions",
                &format!("cursor lookup {} failed: {}", plan.path.display(), error),
            );
            return PreparedFileResult::Commit;
        }
    }

    let event_index = match super::session_identity::index_events(
        &plan.transcript_path,
        u64::try_from(size_bytes).unwrap_or(u64::MAX),
    ) {
        Ok(index) => index,
        Err(error) => {
            summary.failed_files += 1;
            crate::log::error(
                "ingest-sessions",
                &format!("index {} failed: {error}", plan.path.display()),
            );
            return PreparedFileResult::Commit;
        }
    };
    let drain_options = TranscriptDrainOptions {
        source_root: &root.label,
        tolerate_partial_tail: now - mtime_epoch <= ACTIVE_TAIL_WINDOW_SECS,
        transcript_identity_id: Some(identity.id),
    };

    match raw_archive::drain_transcript_with_capture_limit(
        conn,
        &plan.transcript_path,
        &identity.canonical_session_id,
        &identity.project,
        plan.branch.as_deref(),
        plan.cwd.as_deref(),
        &drain_options,
        Some(u64::try_from(size_bytes).unwrap_or(u64::MAX)),
    ) {
        Ok(report) => {
            summary.ingested_messages += report.inserted;
            if report.has_failures() {
                // drain_transcript_with_options already recorded the failure
                // in raw_ingest_failures; keep the cursor behind so the file
                // is retried on the next run.
                summary.failed_files += 1;
                crate::log::error(
                    "ingest-sessions",
                    &format!(
                        "file {} failed: kind={} parse_errors={} insert_errors={} read_error={}",
                        plan.path.display(),
                        report.failure_kind().unwrap_or("unknown"),
                        report.parse_errors,
                        report.insert_errors,
                        report.read_error.is_some()
                    ),
                );
                if report.identity_conflicts > 0 {
                    return PreparedFileResult::Rollback {
                        identity_conflict: true,
                    };
                }
            } else if report.partial_tail {
                summary.partial_files += 1;
            } else {
                let completion = (|| -> Result<super::session_identity::RekeyReport> {
                    conn.execute_batch("SAVEPOINT gh871_identity_complete")?;
                    let rekey = super::session_identity::rekey_legacy_rows(conn, &identity)?;
                    super::session_identity::mark_complete(conn, identity.id, event_index, now)?;
                    advance_cursor(conn, root, &plan.path, mtime_epoch, size_bytes, now)?;
                    conn.execute_batch("RELEASE gh871_identity_complete")?;
                    Ok(rekey)
                })();
                match completion {
                    Ok(rekey) => {
                        summary.ingested_messages =
                            summary.ingested_messages.saturating_sub(rekey.merged);
                    }
                    Err(error) => {
                        if let Err(rollback_error) = conn.execute_batch(
                            "ROLLBACK TO gh871_identity_complete; RELEASE gh871_identity_complete",
                        ) {
                            crate::log::error(
                                "ingest-sessions",
                                &format!(
                                    "identity completion rollback {} failed: {rollback_error}",
                                    plan.path.display()
                                ),
                            );
                        }
                        summary.failed_files += 1;
                        crate::log::error(
                            "ingest-sessions",
                            &format!(
                                "identity completion {} failed: {error}",
                                plan.path.display()
                            ),
                        );
                        return PreparedFileResult::Rollback {
                            identity_conflict: error
                                .downcast_ref::<crate::memory::raw_occurrence::RawIdentityConflict>(
                                )
                                .is_some(),
                        };
                    }
                }
            }
        }
        Err(error) => {
            summary.failed_files += 1;
            crate::log::error(
                "ingest-sessions",
                &format!("drain {} failed: {}", plan.path.display(), error),
            );
        }
    }
    PreparedFileResult::Commit
}

fn cursor_unchanged(
    conn: &Connection,
    root: &ScanRoot,
    file: &Path,
    mtime_epoch: i64,
    size_bytes: i64,
) -> Result<bool> {
    let key = cursor_key(root, file);
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT mtime_epoch, size_bytes FROM ingest_cursors WHERE file_path = ?1",
            params![key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row == Some((mtime_epoch, size_bytes)))
}

pub(crate) fn cursor_matches_identity(
    conn: &Connection,
    root: &ScanRoot,
    file: &Path,
    observed_mtime_ns: i64,
    observed_size_bytes: i64,
) -> Result<bool> {
    cursor_unchanged(
        conn,
        root,
        file,
        observed_mtime_ns / 1_000_000_000,
        observed_size_bytes,
    )
}

fn advance_cursor(
    conn: &Connection,
    root: &ScanRoot,
    file: &Path,
    mtime_epoch: i64,
    size_bytes: i64,
    now: i64,
) -> Result<()> {
    let key = cursor_key(root, file);
    conn.execute(
        "INSERT INTO ingest_cursors (file_path, mtime_epoch, size_bytes, last_ingested_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(file_path) DO UPDATE SET
             mtime_epoch = excluded.mtime_epoch,
             size_bytes = excluded.size_bytes,
             last_ingested_at = excluded.last_ingested_at",
        params![key, mtime_epoch, size_bytes, now],
    )?;
    Ok(())
}

fn cursor_key(root: &ScanRoot, file: &Path) -> String {
    format!("{}\0{}", root.label, file.to_string_lossy())
}

#[cfg(test)]
#[path = "sessions/tests.rs"]
mod tests;
