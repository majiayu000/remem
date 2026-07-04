//! `remem ingest-sessions` — batch, incremental, idempotent ingestion of
//! Claude Code / Codex session transcripts into `raw_messages` (issue #722).
//!
//! Discovery walks each scan root for `*.jsonl` files (skipping `subagents/`
//! directories), a per-file cursor in `ingest_cursors` skips files whose
//! mtime and size are unchanged, and each hit is drained through the existing
//! `drain_transcript` path so the `raw_messages` UNIQUE constraint dedupes
//! against the Stop-hook ingestion running concurrently.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::memory::raw_archive::{self, TranscriptDrainOptions, SOURCE_ROOT_LOCAL};

/// A file whose mtime is within this many seconds of now is treated as an
/// actively-appended session: a JSON parse failure on its last line is a
/// partial tail, not a file failure, and the cursor does not advance.
const ACTIVE_TAIL_WINDOW_SECS: i64 = 60;

/// How many leading lines to inspect when deriving project/branch/cwd from
/// the transcript content itself.
const CONTEXT_PROBE_LINES: usize = 20;

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

    for root in roots {
        if !root.path.is_dir() {
            if root.required {
                summary.failed_files += 1;
                crate::log::error(
                    "ingest-sessions",
                    &format!(
                        "required scan root {}={} is missing or not a directory",
                        root.label,
                        root.path.display()
                    ),
                );
            }
            continue;
        }
        let mut files = Vec::new();
        let mut discovery_failures = Vec::new();
        collect_jsonl_files(&root.path, &mut files, &mut discovery_failures);
        for failure in discovery_failures {
            summary.failed_files += 1;
            crate::log::error("ingest-sessions", &failure);
        }
        files.sort();
        for file in files {
            summary.scanned += 1;
            ingest_one_file(conn, root, &file, options, now, &mut summary);
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

fn ingest_one_file(
    conn: &Connection,
    root: &ScanRoot,
    file: &Path,
    options: &IngestOptions,
    now: i64,
    summary: &mut IngestSummary,
) {
    let (mtime_epoch, size_bytes) = match file_stat(file) {
        Ok(stat) => stat,
        Err(error) => {
            summary.failed_files += 1;
            crate::log::error(
                "ingest-sessions",
                &format!("stat {} failed: {}", file.display(), error),
            );
            return;
        }
    };

    if let Some(since) = options.since_epoch {
        if mtime_epoch < since {
            summary.skipped += 1;
            return;
        }
    }
    match cursor_unchanged(conn, root, file, mtime_epoch, size_bytes) {
        Ok(true) => {
            summary.skipped += 1;
            return;
        }
        Ok(false) => {}
        Err(error) => {
            summary.failed_files += 1;
            crate::log::error(
                "ingest-sessions",
                &format!("cursor lookup {} failed: {}", file.display(), error),
            );
            return;
        }
    }

    let fallback_session_id = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    let context = probe_transcript_context(file);
    let session_id = context
        .session_id
        .as_deref()
        .unwrap_or(&fallback_session_id);
    let project = context
        .cwd
        .as_deref()
        .map(crate::project_id::project_from_cwd)
        .unwrap_or_else(|| fallback_project_slug(root, file));
    let drain_options = TranscriptDrainOptions {
        source_root: &root.label,
        tolerate_partial_tail: now - mtime_epoch <= ACTIVE_TAIL_WINDOW_SECS,
    };

    match raw_archive::drain_transcript_with_options(
        conn,
        &file.to_string_lossy(),
        session_id,
        &project,
        context.branch.as_deref(),
        context.cwd.as_deref(),
        &drain_options,
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
                        file.display(),
                        report.failure_kind().unwrap_or("unknown"),
                        report.parse_errors,
                        report.insert_errors,
                        report.read_error.is_some()
                    ),
                );
            } else if report.partial_tail {
                summary.partial_files += 1;
            } else if let Err(error) =
                advance_cursor(conn, root, file, mtime_epoch, size_bytes, now)
            {
                summary.failed_files += 1;
                crate::log::error(
                    "ingest-sessions",
                    &format!("cursor advance {} failed: {}", file.display(), error),
                );
            }
        }
        Err(error) => {
            summary.failed_files += 1;
            crate::log::error(
                "ingest-sessions",
                &format!("drain {} failed: {}", file.display(), error),
            );
        }
    }
}

fn file_stat(file: &Path) -> Result<(i64, i64)> {
    let metadata = std::fs::metadata(file)?;
    let mtime_epoch = metadata
        .modified()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    Ok((mtime_epoch, metadata.len() as i64))
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

#[derive(Debug, Clone, Default)]
struct TranscriptContext {
    session_id: Option<String>,
    cwd: Option<String>,
    branch: Option<String>,
}

/// Derive project identity inputs from the transcript itself so batch rows
/// dedupe against Stop-hook rows for the same session. Claude Code lines
/// carry top-level `cwd`/`gitBranch`; Codex rollouts carry canonical
/// `payload.id` plus `payload.cwd`/`payload.git.branch` on `session_meta`.
fn probe_transcript_context(file: &Path) -> TranscriptContext {
    let mut context = TranscriptContext::default();
    let Ok(handle) = std::fs::File::open(file) else {
        return context;
    };
    let reader = std::io::BufReader::new(handle);
    for line in reader.lines().take(CONTEXT_PROBE_LINES) {
        let Ok(line) = line else {
            break;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let payload = value.get("payload");
        if context.session_id.is_none() {
            context.session_id = value
                .get("sessionId")
                .and_then(serde_json::Value::as_str)
                .or_else(|| value.get("session_id").and_then(serde_json::Value::as_str))
                .or_else(|| {
                    (value.get("type").and_then(serde_json::Value::as_str) == Some("session_meta"))
                        .then_some(())
                        .and_then(|_| {
                            payload
                                .and_then(|p| p.get("id"))
                                .and_then(serde_json::Value::as_str)
                        })
                })
                .map(str::to_string);
        }
        if context.cwd.is_none() {
            context.cwd = value
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .or_else(|| payload.and_then(|p| p.get("cwd")).and_then(|v| v.as_str()))
                .map(str::to_string);
        }
        if context.branch.is_none() {
            context.branch = value
                .get("gitBranch")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    payload
                        .and_then(|p| p.get("git"))
                        .and_then(|g| g.get("branch"))
                        .and_then(|v| v.as_str())
                })
                .map(str::to_string);
        }
        if context.session_id.is_some() && context.cwd.is_some() && context.branch.is_some() {
            break;
        }
    }
    context
}

/// When the transcript carries no cwd, fall back to the directory slug
/// relative to the scan root (e.g. the Claude project slug, or the
/// `YYYY/MM/DD` bucket for Codex rollouts).
fn fallback_project_slug(root: &ScanRoot, file: &Path) -> String {
    let parent = file.parent().unwrap_or(&root.path);
    let relative = parent.strip_prefix(&root.path).unwrap_or(parent);
    let slug = relative.to_string_lossy();
    if slug.is_empty() {
        root.label.clone()
    } else {
        slug.to_string()
    }
}

#[cfg(test)]
#[path = "sessions/tests.rs"]
mod tests;
