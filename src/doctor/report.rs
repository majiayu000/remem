use std::io::{self, Write};

use anyhow::Result;

use super::database::{check_database, check_disk_space, check_pending_queue, check_worker_daemon};
use super::environment::{check_binary, check_hooks, check_mcp};
use super::schema::check_schema_migration;
use super::types::{Check, CheckJson, DoctorOutcome, ReportJson, Status, REPORT_SCHEMA_VERSION};

/// Caller-supplied options for `remem doctor`. Defaulting all fields keeps
/// the unit tests and any future callers small while letting `cli::dispatch`
/// thread the user-facing CLI flags through.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DoctorOptions {
    /// Emit a single JSON object instead of human-readable lines.
    pub json: bool,
    /// Suppress the per-check lines and the trailing summary in human mode.
    /// Has no effect on `json` (the JSON output is the API surface).
    pub quiet: bool,
}

pub(crate) fn run_doctor(opts: DoctorOptions) -> Result<DoctorOutcome> {
    let stdout = io::stdout();
    let mut sink = stdout.lock();
    run_doctor_with_writer(opts, &mut sink)
}

/// Internal entry point that takes a writer so tests can assert on output
/// without spawning a subprocess.
pub(crate) fn run_doctor_with_writer<W: Write>(
    opts: DoctorOptions,
    out: &mut W,
) -> Result<DoctorOutcome> {
    let checks = collect_checks();
    let outcome = tally(&checks);

    if opts.json {
        write_json(out, &checks, outcome)?;
    } else if !opts.quiet {
        write_human(out, &checks, outcome)?;
    }

    Ok(outcome)
}

fn collect_checks() -> Vec<Check> {
    let mut checks = vec![check_binary(), check_schema_migration(), check_database()];
    checks.extend(check_hooks());
    checks.extend(check_mcp());
    checks.push(check_worker_daemon());
    checks.push(check_pending_queue());
    checks.push(check_disk_space());
    checks
}

fn tally(checks: &[Check]) -> DoctorOutcome {
    let mut outcome = DoctorOutcome::default();
    for check in checks {
        match check.status {
            Status::Warn => outcome.warns += 1,
            Status::Fail => outcome.fails += 1,
            Status::Ok => {}
        }
    }
    outcome
}

fn write_human<W: Write>(out: &mut W, checks: &[Check], outcome: DoctorOutcome) -> Result<()> {
    writeln!(
        out,
        "remem v{} — system check",
        crate::build_info::version_label()
    )?;
    writeln!(out)?;

    for check in checks {
        writeln!(out, "  [{}] {}: {}", check.icon(), check.name, check.detail)?;
    }

    writeln!(out)?;
    if outcome.fails > 0 {
        writeln!(
            out,
            "{} check(s) failed, {} warning(s). Run `remem install` to fix hook/MCP issues.",
            outcome.fails, outcome.warns
        )?;
    } else if outcome.warns > 0 {
        writeln!(out, "All checks passed with {} warning(s).", outcome.warns)?;
    } else {
        writeln!(out, "All checks passed.")?;
    }

    Ok(())
}

fn write_json<W: Write>(out: &mut W, checks: &[Check], outcome: DoctorOutcome) -> Result<()> {
    let overall = if outcome.fails > 0 {
        Status::Fail
    } else if outcome.warns > 0 {
        Status::Warn
    } else {
        Status::Ok
    };

    let report = ReportJson {
        schema_version: REPORT_SCHEMA_VERSION,
        version: crate::build_info::package_version(),
        binary_schema_version: crate::build_info::binary_schema_version(),
        status: overall.as_json_tag(),
        fails: outcome.fails,
        warns: outcome.warns,
        checks: checks
            .iter()
            .map(|c| CheckJson {
                name: c.name,
                status: c.status.as_json_tag(),
                detail: c.detail.as_str(),
            })
            .collect(),
    };

    let serialized = serde_json::to_string(&report)?;
    writeln!(out, "{serialized}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(name: &'static str, status: Status, detail: &str) -> Check {
        Check {
            name,
            status,
            detail: detail.into(),
        }
    }

    #[test]
    fn tally_counts_each_status() {
        let checks = [
            make("a", Status::Ok, "fine"),
            make("b", Status::Warn, "soft"),
            make("c", Status::Fail, "hard"),
            make("d", Status::Fail, "harder"),
        ];
        let outcome = tally(&checks);
        assert_eq!(outcome.fails, 2);
        assert_eq!(outcome.warns, 1);
        assert_eq!(outcome.exit_code(), 2);
    }

    #[test]
    fn tally_only_warnings_yields_exit_one() {
        let checks = [
            make("a", Status::Ok, "fine"),
            make("b", Status::Warn, "soft"),
        ];
        let outcome = tally(&checks);
        assert_eq!(outcome.exit_code(), 1);
    }

    #[test]
    fn tally_all_ok_yields_exit_zero() {
        let checks = [make("a", Status::Ok, "fine"), make("b", Status::Ok, "also")];
        let outcome = tally(&checks);
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn human_output_lists_each_check_and_failure_summary() {
        let checks = vec![
            make("a", Status::Ok, "fine"),
            make("b", Status::Fail, "broken"),
        ];
        let outcome = tally(&checks);
        let mut buf = Vec::new();
        write_human(&mut buf, &checks, outcome).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("[ok] a: fine"));
        assert!(text.contains("[FAIL] b: broken"));
        assert!(text.contains("1 check(s) failed"));
    }

    #[test]
    fn json_output_is_machine_parseable() {
        let checks = vec![
            make("Database", Status::Ok, "0.1 MB, 0 memories"),
            make("Hooks", Status::Fail, "missing"),
        ];
        let outcome = tally(&checks);
        let mut buf = Vec::new();
        write_json(&mut buf, &checks, outcome).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(
            parsed["binary_schema_version"],
            crate::migrate::latest_schema_version()
        );
        assert_eq!(parsed["status"], "fail");
        assert_eq!(parsed["fails"], 1);
        assert_eq!(parsed["warns"], 0);
        let checks_json = parsed["checks"].as_array().unwrap();
        assert_eq!(checks_json.len(), 2);
        assert_eq!(checks_json[0]["name"], "Database");
        assert_eq!(checks_json[0]["status"], "ok");
        assert_eq!(checks_json[1]["status"], "fail");
    }

    #[test]
    fn json_output_for_all_ok_reports_status_ok_and_zero_counts() {
        let checks = vec![
            make("Binary", Status::Ok, "ok"),
            make("Database", Status::Ok, "ok"),
        ];
        let outcome = tally(&checks);
        let mut buf = Vec::new();
        write_json(&mut buf, &checks, outcome).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(String::from_utf8(buf).unwrap().trim()).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["fails"], 0);
        assert_eq!(parsed["warns"], 0);
    }

    #[test]
    fn json_wins_over_quiet_when_both_set() {
        // The contract: --json is the API surface, --quiet only suppresses
        // the human formatter. With both flags the JSON object must still
        // be emitted (otherwise scripts using `remem doctor --json --quiet`
        // would see empty stdout).
        let opts = DoctorOptions {
            json: true,
            quiet: true,
        };
        let checks = vec![make("Database", Status::Ok, "ok")];
        let outcome = tally(&checks);
        let mut buf = Vec::new();
        if opts.json {
            write_json(&mut buf, &checks, outcome).unwrap();
        } else if !opts.quiet {
            write_human(&mut buf, &checks, outcome).unwrap();
        }
        assert!(!buf.is_empty(), "json must win over quiet");
        let text = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(parsed["status"], "ok");
    }

    #[test]
    fn quiet_human_mode_emits_no_output_but_returns_outcome() {
        let opts = DoctorOptions {
            json: false,
            quiet: true,
        };
        // We can't drive collect_checks() in a test (it touches the real DB);
        // exercise just the writer dispatch with a synthetic checks slice.
        let checks = vec![make("a", Status::Fail, "broken")];
        let outcome = tally(&checks);
        let mut buf = Vec::new();
        if opts.json {
            write_json(&mut buf, &checks, outcome).unwrap();
        } else if !opts.quiet {
            write_human(&mut buf, &checks, outcome).unwrap();
        }
        assert!(buf.is_empty(), "quiet mode must not write to stdout");
        assert_eq!(outcome.fails, 1);
        assert_eq!(outcome.exit_code(), 2);
    }
}
