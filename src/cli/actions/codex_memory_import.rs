//! `remem import codex-memories` — one-way, read-only import of Codex CLI
//! native rollout-summary memories into the candidate review queue (GH-852).
//!
//! Contract (specs/GH852): source files are untrusted external content. The
//! import is two-phase (discover + freeze plan, then apply bound to
//! `--expect-plan-digest`), fails closed on any unknown/malformed/secret
//! input, never touches the source tree, and lands records only in
//! `pending_review` / `quarantined` — never directly in active memories.

mod apply;
mod discovery;
mod parser;
mod plan;

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use discovery::{discover_source, SourceDiscovery};
use plan::{build_plan, ImportPlan};

pub(in crate::cli) fn run_import_codex_memories(
    source: Option<&Path>,
    dry_run: bool,
    expect_plan_digest: Option<&str>,
) -> Result<()> {
    let source_dir = resolve_source_dir(source);

    let discovery = match discover_source(&source_dir)? {
        SourceDiscovery::NotConfigured => {
            println!(
                "{}",
                serde_json::json!({
                    "source_state": "not_configured",
                    "source": redact_home(&source_dir),
                    "detail": "no Codex native memories found; nothing to import",
                })
            );
            return Ok(());
        }
        SourceDiscovery::Ready(files) => files,
    };

    let conn = crate::db::open_db()?;
    let plan = build_plan(&conn, &discovery)?;

    if dry_run {
        println!("{}", plan_report(&plan, "dry_run"));
        return Ok(());
    }

    let Some(expected_digest) = expect_plan_digest else {
        bail!(
            "codex-memories apply requires --expect-plan-digest <sha256> from a prior --dry-run; \
             re-run with --dry-run first"
        );
    };
    if plan.secret_blocked > 0 {
        bail!(
            "codex-memories apply blocked: {} file(s) contain secret-like content; \
             nothing was persisted",
            plan.secret_blocked
        );
    }
    if plan.plan_digest != expected_digest {
        bail!(
            "codex-memories plan digest mismatch: expected {} but current source planning produced {}; \
             the source or classification changed — re-run --dry-run and review the new plan",
            expected_digest,
            plan.plan_digest
        );
    }

    let summary = apply::apply_plan(conn, &plan)?;
    println!("{}", plan_report_applied(&plan, &summary));
    Ok(())
}

fn resolve_source_dir(source: Option<&Path>) -> PathBuf {
    match source {
        Some(path) => path.to_path_buf(),
        None => crate::install::codex_memories_dir(),
    }
}

/// B-014: diagnostics must not leak absolute paths with sensitive usernames.
pub(crate) fn redact_home(path: &Path) -> String {
    let display = path.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home = home.display().to_string();
        if let Some(rest) = display.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    display
}

fn plan_report(plan: &ImportPlan, mode: &str) -> String {
    serde_json::json!({
        "source_state": plan.source_state(),
        "mode": mode,
        "format_versions": plan.format_versions(),
        "files": plan.file_ids(),
        "planned_import": plan.planned_import(),
        "dedup": plan.dedup(),
        "quarantine": plan.quarantine(),
        "secret_blocked": plan.secret_blocked,
        "plan_digest": if plan.secret_blocked > 0 { serde_json::Value::Null } else { serde_json::Value::String(plan.plan_digest.clone()) },
    })
    .to_string()
}

fn plan_report_applied(plan: &ImportPlan, summary: &apply::ApplySummary) -> String {
    serde_json::json!({
        "source_state": plan.source_state(),
        "mode": "apply",
        "format_versions": plan.format_versions(),
        "imported_pending_review": summary.pending_review,
        "imported_quarantined": summary.quarantined,
        "dedup": summary.dedup,
        "plan_digest": plan.plan_digest,
    })
    .to_string()
}

#[cfg(test)]
mod tests;
