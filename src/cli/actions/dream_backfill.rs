use std::{collections::BTreeMap, fmt::Write as _};

use anyhow::{bail, Result};

use crate::cli::types::DreamBackfillArgs;
use crate::db;
use crate::dream::backfill::{run_backfill_with_expected_plan_digest, BackfillReport};

pub(in crate::cli) fn run_dream_backfill(args: DreamBackfillArgs) -> Result<()> {
    let dry_run = resolve_dry_run(args.apply, args.dry_run)?;
    let mut conn = db::open_db()?;
    let report = run_backfill_with_expected_plan_digest(
        &mut conn,
        dry_run,
        args.expect_plan_digest.as_deref(),
    )?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    print_report(&report);
    Ok(())
}

fn resolve_dry_run(apply: bool, dry_run: bool) -> Result<bool> {
    if apply && dry_run {
        bail!("--apply and --dry-run are mutually exclusive");
    }
    Ok(!apply)
}

#[derive(Default)]
struct ProjectCounts {
    stock: usize,
    hits: usize,
    trust_only: usize,
    skipped: usize,
}

fn print_report(report: &BackfillReport) {
    print!("{}", render_report(report));
}

fn render_report(report: &BackfillReport) -> String {
    let plan = &report.plan;
    let mut output = String::new();
    let mut by_project: BTreeMap<String, ProjectCounts> = BTreeMap::new();
    for hit in &plan.hits {
        let counts = by_project.entry(hit.project.clone()).or_default();
        counts.stock += 1;
        counts.hits += 1;
    }
    for no_hit in &plan.no_hits {
        let counts = by_project.entry(no_hit.project.clone()).or_default();
        counts.stock += 1;
        counts.trust_only += 1;
    }
    for skip in &plan.skipped {
        let counts = by_project.entry(skip.project.clone()).or_default();
        counts.stock += 1;
        counts.skipped += 1;
    }

    if report.dry_run {
        writeln!(output, "Dream backfill dry-run (GH-990):").unwrap();
    } else {
        writeln!(output, "Dream backfill applied (GH-990):").unwrap();
    }
    writeln!(output, "  Plan digest: {}", report.plan_digest).unwrap();
    writeln!(
        output,
        "  Stock (Dream-merged, active, pre-v076 trust): {}",
        plan.stock_total
    )
    .unwrap();
    writeln!(output, "  Scanner hits to quarantine: {}", plan.hits.len()).unwrap();
    writeln!(
        output,
        "  Trust-class backfill only: {}",
        plan.no_hits.len()
    )
    .unwrap();
    if !plan.skipped.is_empty() {
        writeln!(
            output,
            "  Skipped (cannot satisfy artifact constraints): {}",
            plan.skipped.len()
        )
        .unwrap();
    }
    if !by_project.is_empty() {
        writeln!(output, "  By project:").unwrap();
        for (project, counts) in &by_project {
            writeln!(
                output,
                "    {project}: stock={} hits={} trust-only={} skipped={}",
                counts.stock, counts.hits, counts.trust_only, counts.skipped
            )
            .unwrap();
        }
    }
    if !plan.hits.is_empty() {
        writeln!(output, "  Hits:").unwrap();
        for hit in plan.hits.iter().take(20) {
            writeln!(
                output,
                "    memory {} {:?} — field={} pattern={}@v{}",
                hit.memory_id, hit.title, hit.matched_field, hit.pattern_id, hit.pattern_version
            )
            .unwrap();
        }
        if plan.hits.len() > 20 {
            writeln!(output, "    … and {} more", plan.hits.len() - 20).unwrap();
        }
    }
    for skip in &plan.skipped {
        writeln!(
            output,
            "  Skipped memory {}: {}",
            skip.memory_id, skip.reason
        )
        .unwrap();
    }
    match &report.applied {
        None => {
            writeln!(output, "  No changes written.").unwrap();
            writeln!(
                output,
                "  Re-run with --apply to execute; quarantine ledger rows are immutable once written."
            )
            .unwrap();
        }
        Some(applied) => {
            writeln!(
                output,
                "  Applied: {} quarantined, {} trust-class backfilled.",
                applied.quarantined, applied.trust_backfilled
            )
            .unwrap();
            writeln!(
                output,
                "  Quarantined memories are archived pending review; approving their candidates restores them."
            )
            .unwrap();
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dream::backfill::BackfillPlan;

    #[test]
    fn command_mode_defaults_to_dry_run_and_rejects_ambiguous_flags() {
        assert!(resolve_dry_run(false, false).unwrap());
        assert!(!resolve_dry_run(true, false).unwrap());
        assert!(resolve_dry_run(true, true).is_err());
    }

    #[test]
    fn human_and_json_reports_expose_rehearsal_state_and_plan_binding() {
        let report = BackfillReport {
            dry_run: true,
            plan_digest: "a".repeat(64),
            plan: BackfillPlan::default(),
            applied: None,
        };
        let human = render_report(&report);
        assert!(human.contains("dry-run"));
        assert!(human.contains(&report.plan_digest));
        assert!(human.contains("No changes written"));

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["dry_run"], true);
        assert_eq!(json["plan_digest"], report.plan_digest);
        assert!(json["applied"].is_null());
    }
}
