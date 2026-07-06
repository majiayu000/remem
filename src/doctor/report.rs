use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use anyhow::Result;
use rusqlite::Connection;

use super::capture_capability::check_capture_capabilities;
use super::capture_liveness::check_capture_liveness;
use super::database::{
    check_capture_drops, check_database, check_declared_empty_surfaces, check_disk_space,
    check_legacy_surfaces, check_memory_usage_feedback, check_pending_queue,
    check_promotion_funnel, check_raw_archive_ingest, check_temporal_facts, check_worker_daemon,
};
use super::embedding::check_embedding_provider;
use super::environment::{check_binary, check_hooks, check_install_paths, check_mcp};
use super::logging::check_log_health;
use super::mcp_processes::check_mcp_processes;
use super::memory_poisoning::check_memory_poisoning_defense;
use super::native_memory::check_native_memory_sync;
use super::review_queue::check_review_queue;
use super::runtime_config_check::check_runtime_config;
use super::schema::{check_key_format, check_schema_migration};
use super::types::{Check, CheckJson, DoctorOutcome, ReportJson, Status, REPORT_SCHEMA_VERSION};

const SLOW_CHECK_WARN_MS: u64 = 10_000;

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
    let started = Instant::now();
    if !opts.json && !opts.quiet {
        write_human_header(out)?;
    }

    let checks = run_checks(|check| {
        if !opts.json && !opts.quiet {
            write_human_check(out, check)?;
            out.flush()?;
        }
        Ok(())
    })?;
    let outcome = tally(&checks);

    if opts.json {
        let observability = build_observability_report();
        let elapsed_ms = duration_ms(started.elapsed());
        write_json(out, &checks, outcome, elapsed_ms, observability)?;
    } else if !opts.quiet {
        write_human_summary(out, outcome)?;
    }

    Ok(outcome)
}

fn run_checks(mut on_check: impl FnMut(&Check) -> Result<()>) -> Result<Vec<Check>> {
    let mut checks = Vec::new();

    push_check(&mut checks, &mut on_check, check_binary)?;
    let shared_db = SharedDoctorDb::open();
    push_check(&mut checks, &mut on_check, || {
        check_schema_migration(shared_db.conn(), shared_db.open_error())
    })?;
    push_check(&mut checks, &mut on_check, check_key_format)?;
    push_check(&mut checks, &mut on_check, || {
        check_database(shared_db.conn(), shared_db.open_error())
    })?;
    push_check(&mut checks, &mut on_check, check_install_paths)?;
    push_check(&mut checks, &mut on_check, check_runtime_config)?;
    push_checks(&mut checks, &mut on_check, || {
        check_embedding_provider(shared_db.conn())
    })?;
    push_checks(&mut checks, &mut on_check, check_hooks)?;
    push_checks(&mut checks, &mut on_check, check_capture_capabilities)?;
    push_checks(&mut checks, &mut on_check, check_mcp)?;
    push_check(&mut checks, &mut on_check, check_mcp_processes)?;
    let started = Instant::now();
    let check = check_capture_liveness(shared_db.conn(), &checks)
        .with_duration_ms(duration_ms(started.elapsed()));
    push_ready_check(&mut checks, &mut on_check, check)?;
    push_check(&mut checks, &mut on_check, || {
        check_raw_archive_ingest(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_capture_drops(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_memory_usage_feedback(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_declared_empty_surfaces(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_legacy_surfaces(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_promotion_funnel(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_memory_poisoning_defense(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_review_queue(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_temporal_facts(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_worker_daemon(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, || {
        check_pending_queue(shared_db.conn())
    })?;
    push_check(&mut checks, &mut on_check, check_native_memory_sync)?;
    push_check(&mut checks, &mut on_check, check_log_health)?;
    push_check(&mut checks, &mut on_check, check_disk_space)?;
    Ok(checks)
}

fn push_check(
    checks: &mut Vec<Check>,
    on_check: &mut impl FnMut(&Check) -> Result<()>,
    build: impl FnOnce() -> Check,
) -> Result<()> {
    let started = Instant::now();
    let check = build().with_duration_ms(duration_ms(started.elapsed()));
    push_ready_check(checks, on_check, check)
}

fn push_checks(
    checks: &mut Vec<Check>,
    on_check: &mut impl FnMut(&Check) -> Result<()>,
    build: impl FnOnce() -> Vec<Check>,
) -> Result<()> {
    let started = Instant::now();
    let built = build();
    let duration = duration_ms(started.elapsed());
    for check in built {
        push_ready_check(checks, on_check, check.with_duration_ms(duration))?;
    }
    Ok(())
}

fn push_ready_check(
    checks: &mut Vec<Check>,
    on_check: &mut impl FnMut(&Check) -> Result<()>,
    check: Check,
) -> Result<()> {
    if check.duration_ms > SLOW_CHECK_WARN_MS {
        crate::log::warn(
            "doctor",
            &format!("slow check '{}' took {}ms", check.name, check.duration_ms),
        );
    }
    on_check(&check)?;
    checks.push(check);
    Ok(())
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

struct SharedDoctorDb {
    conn: Option<Connection>,
    open_error: Option<String>,
}

impl SharedDoctorDb {
    fn open() -> Self {
        if !crate::db::db_path().exists() {
            return Self {
                conn: None,
                open_error: None,
            };
        }

        match crate::db::open_db_read_only() {
            Ok(conn) => Self {
                conn: Some(conn),
                open_error: None,
            },
            Err(error) => Self {
                conn: None,
                open_error: Some(error.to_string()),
            },
        }
    }

    fn conn(&self) -> Option<&Connection> {
        self.conn.as_ref()
    }

    fn open_error(&self) -> Option<&str> {
        self.open_error.as_deref()
    }
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

#[cfg(test)]
fn write_human<W: Write>(out: &mut W, checks: &[Check], outcome: DoctorOutcome) -> Result<()> {
    write_human_header(out)?;
    for check in checks {
        write_human_check(out, check)?;
    }
    write_human_summary(out, outcome)
}

fn write_human_header<W: Write>(out: &mut W) -> Result<()> {
    writeln!(
        out,
        "remem v{} — system check",
        crate::build_info::version_label()
    )?;
    writeln!(out)?;
    Ok(())
}

fn write_human_check<W: Write>(out: &mut W, check: &Check) -> Result<()> {
    writeln!(
        out,
        "  [{}] {}: {} ({}ms)",
        check.icon(),
        check.name,
        check.detail,
        check.duration_ms
    )?;
    Ok(())
}

fn write_human_summary<W: Write>(out: &mut W, outcome: DoctorOutcome) -> Result<()> {
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

fn write_json<W: Write>(
    out: &mut W,
    checks: &[Check],
    outcome: DoctorOutcome,
    elapsed_ms: u64,
    observability: crate::db::ObservabilityReport,
) -> Result<()> {
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
        elapsed_ms,
        checks: checks
            .iter()
            .map(|c| CheckJson {
                name: c.name,
                status: c.status.as_json_tag(),
                detail: c.detail.as_str(),
                duration_ms: c.duration_ms,
            })
            .collect(),
        observability,
    };

    let serialized = serde_json::to_string(&report)?;
    writeln!(out, "{serialized}")?;
    Ok(())
}

fn build_observability_report() -> crate::db::ObservabilityReport {
    let generated_at_epoch = chrono::Utc::now().timestamp();
    if !crate::db::db_path().exists() {
        return crate::db::ObservabilityReport::unavailable(
            generated_at_epoch,
            "remem database does not exist",
        );
    }
    match crate::db::open_db_read_only() {
        Ok(conn) => crate::db::query_observability_report(&conn, generated_at_epoch)
            .unwrap_or_else(|error| {
                crate::db::ObservabilityReport::unavailable(generated_at_epoch, error.to_string())
            }),
        Err(error) => {
            crate::db::ObservabilityReport::unavailable(generated_at_epoch, error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(name: &'static str, status: Status, detail: &str) -> Check {
        Check::new(name, status, detail)
    }

    fn test_observability_report() -> crate::db::ObservabilityReport {
        crate::db::ObservabilityReport::unavailable(0, "test")
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
    fn json_output_is_machine_parseable() -> anyhow::Result<()> {
        let checks = vec![
            make("Database", Status::Ok, "0.1 MB, 0 memories"),
            make("Hooks", Status::Fail, "missing"),
        ];
        let outcome = tally(&checks);
        let mut buf = Vec::new();
        write_json(&mut buf, &checks, outcome, 123, test_observability_report())?;
        let text = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(parsed["schema_version"], 3);
        assert_eq!(
            parsed["binary_schema_version"],
            crate::migrate::latest_schema_version()
        );
        assert_eq!(parsed["status"], "fail");
        assert_eq!(parsed["fails"], 1);
        assert_eq!(parsed["warns"], 0);
        assert_eq!(parsed["elapsed_ms"], 123);
        assert_eq!(parsed["observability"]["schema_version"], 1);
        assert_eq!(
            parsed["observability"]["spec_path"],
            "docs/specs/current-memory-contracts/TECH.md"
        );
        assert!(parsed["observability"]["checks"].is_array());
        assert!(parsed["observability"]["metrics"]["capture"]["captured_events"].is_i64());
        let checks_json = parsed["checks"].as_array().unwrap();
        assert_eq!(checks_json.len(), 2);
        assert_eq!(checks_json[0]["name"], "Database");
        assert_eq!(checks_json[0]["status"], "ok");
        assert_eq!(checks_json[0]["duration_ms"], 0);
        assert_eq!(checks_json[1]["status"], "fail");
        Ok(())
    }

    #[test]
    fn json_output_for_all_ok_reports_status_ok_and_zero_counts() -> anyhow::Result<()> {
        let checks = vec![
            make("Binary", Status::Ok, "ok"),
            make("Database", Status::Ok, "ok"),
        ];
        let outcome = tally(&checks);
        let mut buf = Vec::new();
        write_json(&mut buf, &checks, outcome, 0, test_observability_report())?;
        let parsed: serde_json::Value =
            serde_json::from_str(String::from_utf8(buf).unwrap().trim()).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["fails"], 0);
        assert_eq!(parsed["warns"], 0);
        Ok(())
    }

    #[test]
    fn push_check_invokes_callback_before_next_check_runs() -> anyhow::Result<()> {
        let events = std::cell::RefCell::new(Vec::new());
        let mut checks = Vec::new();

        push_check(
            &mut checks,
            &mut |check| {
                events.borrow_mut().push(format!("write {}", check.name));
                Ok(())
            },
            || {
                events.borrow_mut().push("build first".to_string());
                Check::new("first", Status::Ok, "ok")
            },
        )?;

        push_check(
            &mut checks,
            &mut |check| {
                events.borrow_mut().push(format!("write {}", check.name));
                Ok(())
            },
            || {
                assert_eq!(
                    *events.borrow(),
                    vec!["build first".to_string(), "write first".to_string()]
                );
                Check::new("second", Status::Ok, "ok")
            },
        )?;

        assert_eq!(checks.len(), 2);
        Ok(())
    }

    #[test]
    fn push_checks_assigns_measured_duration_to_each_check() -> anyhow::Result<()> {
        let mut checks = Vec::new();
        let mut observed = Vec::new();

        push_checks(
            &mut checks,
            &mut |check| {
                observed.push((check.name, check.duration_ms));
                Ok(())
            },
            || {
                std::thread::sleep(Duration::from_millis(5));
                vec![
                    make("first", Status::Ok, "ok"),
                    make("second", Status::Ok, "ok"),
                ]
            },
        )?;

        assert_eq!(observed.len(), 2);
        assert_eq!(checks.len(), 2);
        assert!(observed.iter().all(|(_, duration)| *duration > 0));
        assert!(checks.iter().all(|check| check.duration_ms > 0));
        Ok(())
    }

    #[test]
    fn json_wins_over_quiet_when_both_set() -> anyhow::Result<()> {
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
            write_json(&mut buf, &checks, outcome, 0, test_observability_report())?;
        } else if !opts.quiet {
            write_human(&mut buf, &checks, outcome).unwrap();
        }
        assert!(!buf.is_empty(), "json must win over quiet");
        let text = String::from_utf8(buf).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(parsed["status"], "ok");
        Ok(())
    }

    #[test]
    fn quiet_human_mode_emits_no_output_but_returns_outcome() -> anyhow::Result<()> {
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
            write_json(&mut buf, &checks, outcome, 0, test_observability_report())?;
        } else if !opts.quiet {
            write_human(&mut buf, &checks, outcome).unwrap();
        }
        assert!(buf.is_empty(), "quiet mode must not write to stdout");
        assert_eq!(outcome.fails, 1);
        assert_eq!(outcome.exit_code(), 2);
        Ok(())
    }
}
