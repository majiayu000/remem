use anyhow::Result;
use serde_json::json;

use super::codex_usage::parse_codex_json_events;
use super::UsageStatus;

fn codex(value: serde_json::Value) -> super::UsageObservation {
    let event = json!({"type": "turn.completed", "usage": value});
    parse_codex_json_events(event.to_string().as_bytes(), None)
        .unwrap()
        .unwrap()
        .usage
}

#[test]
fn usage_observation_distinguishes_zero_missing_partial_and_invalid() {
    let zero = codex(
        json!({"input_tokens":0,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}),
    );
    assert_eq!(zero.status, UsageStatus::Complete);
    assert_eq!(zero.known_total_tokens, 0);
    assert!(zero.has_reported_counters);
    assert_eq!(codex(json!(null)).status, UsageStatus::Missing);
    assert_eq!(codex(json!({})).status, UsageStatus::Missing);
    for value in [
        json!({"input_tokens":100}),
        json!({"input_tokens":100,"output_tokens":null}),
    ] {
        let usage = codex(value);
        assert_eq!(usage.status, UsageStatus::Partial);
        assert_eq!(usage.tokens.raw_input_tokens, 100);
        assert_eq!(usage.known_total_tokens, 100);
        assert!(usage.missing_fields.contains(&"raw_output_tokens"));
    }
    for value in [
        json!({"input_tokens":100,"output_tokens":"40"}),
        json!({"input_tokens":100,"output_tokens":-1}),
        json!({"input_tokens":100,"output_tokens":1.5}),
        json!({"input_tokens":100,"output_tokens":true}),
        json!({"input_tokens":100,"output_tokens":18446744073709551615_u64}),
    ] {
        let usage = codex(value);
        assert_eq!(usage.status, UsageStatus::Invalid);
        assert_eq!(usage.tokens.raw_input_tokens, 100);
        assert!(usage.invalid_fields.contains(&"raw_output_tokens"));
    }
}

#[test]
fn usage_observation_rejects_inconsistent_subtotals_aliases_and_overflow() {
    for value in [
        json!({"input_tokens":10,"cached_input_tokens":11,"output_tokens":0,"reasoning_output_tokens":0}),
        json!({"input_tokens":10,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":2}),
        json!({"input_tokens":10,"cached_input_tokens":1,"cache_read_input_tokens":2,"output_tokens":0,"reasoning_output_tokens":0}),
        json!({"input_tokens":i64::MAX,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}),
    ] {
        assert_eq!(codex(value).status, UsageStatus::Invalid);
    }
    let valid = json!({"type":"turn.completed","usage":{"input_tokens":i64::MAX,"cached_input_tokens":0,"output_tokens":0,"reasoning_output_tokens":0}}).to_string();
    let aggregate = parse_codex_json_events(format!("{valid}\n{valid}").as_bytes(), None)
        .unwrap()
        .unwrap()
        .usage;
    assert_eq!(aggregate.status, UsageStatus::Invalid);
    assert!(aggregate.invalid_fields.contains(&"raw_input_tokens"));
}

#[test]
fn usage_observation_keeps_partial_turn_evidence_additive() {
    let events = concat!(
        "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"output_tokens\":0,\"reasoning_output_tokens\":0}}\n",
        "{\"type\":\"turn.completed\",\"usage\":{\"cached_input_tokens\":20,\"output_tokens\":0,\"reasoning_output_tokens\":0}}\n",
    );
    let usage = parse_codex_json_events(events.as_bytes(), None)
        .unwrap()
        .unwrap()
        .usage;
    assert_eq!(usage.status, UsageStatus::Partial);
    assert_eq!(usage.known_total_tokens, 120);
    assert_eq!(usage.parts.len(), 2);
    assert_eq!(usage.parts[0].tokens.raw_input_tokens, 100);
    assert_eq!(usage.parts[1].tokens.cache_read_tokens, 20);
}

#[test]
fn usage_observation_invalid_subtotals_preserve_raw_gross_in_ledger() -> Result<()> {
    let _scope = crate::db::test_support::ScopedTestDataDir::new("usage-invalid-gross");
    let ctx = super::UsageContext {
        project: Some("/fixture"),
        session_id: None,
        operation: "invalid-usage",
        host: None,
        profile: None,
    };
    for (raw, expected) in [
        (
            json!({"input_tokens":100,"cached_input_tokens":200,"output_tokens":0,"reasoning_output_tokens":0}),
            100,
        ),
        (
            json!({"input_tokens":0,"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":2}),
            1,
        ),
    ] {
        let observation = codex(raw);
        assert_eq!(observation.status, UsageStatus::Invalid);
        assert_eq!(observation.known_total_tokens, expected);
        let evidence = super::types::AiCallResult {
            text: "ok".into(),
            executor: "codex-cli",
            model: "gpt-5.2".into(),
            usage: Some(observation),
            usage_source: Some("codex_log"),
        };
        super::usage::record_usage(ctx, &evidence, "success", 0, 0);
    }
    let conn = crate::db::open_db()?;
    let totals: (i64, i64, i64, f64) = conn.query_row(
        "SELECT COUNT(*), SUM(total_tokens), SUM(usage_status='invalid'), SUM(estimated_cost_usd) FROM ai_usage_events", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
    assert_eq!(totals, (2, 101, 2, 0.0));
    let contradictory_cache: i64 = conn.query_row(
        "SELECT cache_read_tokens FROM ai_usage_events ORDER BY id LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        contradictory_cache, 200,
        "retain contradictory evidence without promoting it to gross usage"
    );
    Ok(())
}

#[cfg(unix)]
fn fake_profile(
    dir: &std::path::Path,
    name: &str,
    ending: &str,
    usage: bool,
) -> Result<crate::runtime_config::ResolvedMemoryAiProfile> {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    let log = if usage {
        "printf '%s\\n' '{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":100,\"cached_input_tokens\":0,\"output_tokens\":40,\"reasoning_output_tokens\":0}}'"
    } else {
        ":"
    };
    let script = format!("#!/bin/sh\nif [ \"$1\" = '--remem-fixture-ready' ]; then exit 0; fi\nwhile [ \"$#\" -gt 0 ]; do\nif [ \"$1\" = '--output-last-message' ]; then shift; output=\"$1\"; fi\nshift\ndone\ncat >/dev/null\n{log}\n{ending}\n");
    std::fs::write(&path, script)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(crate::runtime_config::ResolvedMemoryAiProfile {
        profile_name: "synthetic-usage".into(),
        executor: crate::runtime_config::MemoryAiExecutor::CodexCli,
        model: Some("gpt-5.2".into()),
        cli_path: Some(path.to_string_lossy().into()),
        base_url: None,
        reasoning_effort: None,
    })
}

#[cfg(unix)]
#[test]
fn usage_attempt_records_failed_and_successful_codex_executions_once() -> Result<()> {
    let scoped = crate::db::test_support::ScopedTestDataDir::new("usage-attempts");
    std::fs::create_dir_all(&scoped.path)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let ctx = super::UsageContext {
        project: Some("/synthetic"),
        session_id: Some("fixture"),
        operation: "test-usage",
        host: None,
        profile: None,
    };
    for (name, ending) in [
        ("nonzero", "exit 7"),
        ("empty", ": > \"$output\""),
        ("missing", "exit 0"),
    ] {
        let failure = fake_profile(&scoped.path, name, ending, true)?;
        assert!(runtime
            .block_on(super::with_resolved_profile(
                failure,
                super::call_ai("test", "fixture", ctx)
            ))
            .is_err());
        let success = fake_profile(&scoped.path, "success", "printf 'ok' > \"$output\"", true)?;
        assert_eq!(
            runtime.block_on(super::with_resolved_profile(
                success,
                super::call_ai("test", "fixture", ctx)
            ))?,
            "ok"
        );
    }
    let missing = fake_profile(&scoped.path, "no-usage", "exit 9", false)?;
    assert!(runtime
        .block_on(super::with_resolved_profile(
            missing,
            super::call_ai("test", "fixture", ctx)
        ))
        .is_err());
    let conn = crate::db::open_db()?;
    let stored: (i64, i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*), SUM(total_tokens), SUM(attempt_outcome='failed'), SUM(usage_status='complete') FROM ai_usage_events", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
    assert_eq!(stored, (7, 840, 4, 6));
    let missing: (i64, String) = conn.query_row(
        "SELECT total_tokens, usage_status FROM ai_usage_events ORDER BY id DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(missing, (0, "missing".into()));
    Ok(())
}

#[cfg(unix)]
#[test]
fn usage_attempt_timeout_preserves_terminal_counters_already_read() -> Result<()> {
    let scoped = crate::db::test_support::ScopedTestDataDir::new("usage-timeout");
    std::fs::create_dir_all(&scoped.path)?;
    let profile = fake_profile(
        &scoped.path,
        "timeout",
        "while :; do sleep 0.02; done",
        true,
    )?;
    // Keep cold executable startup outside the timed usage behavior.
    assert!(
        std::process::Command::new(profile.cli_path.as_ref().unwrap())
            .arg("--remem-fixture-ready")
            .status()?
            .success()
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let error = runtime
        .block_on(super::codex_cli::call_codex_cli_with_timeout(
            "test",
            "fixture",
            &profile,
            std::time::Duration::from_millis(250),
            None,
        ))
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    let evidence = &error
        .downcast_ref::<super::types::AiCallFailure>()
        .unwrap()
        .evidence;
    assert_eq!(evidence.usage.as_ref().unwrap().known_total_tokens, 140);
    Ok(())
}

#[cfg(unix)]
#[test]
fn usage_attempt_stdin_failure_and_backpressure_keep_output_evidence() -> Result<()> {
    let scoped = crate::db::test_support::ScopedTestDataDir::new("usage-stdin");
    std::fs::create_dir_all(&scoped.path)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let prompt = "x".repeat(1_000_000);
    for (name, input_action, ending, message) in [
        (
            "closed-input",
            "exec 0<&-",
            "exit 7",
            "failed to write Codex prompt",
        ),
        (
            "unread-input",
            ":",
            "while :; do sleep 0.02; done",
            "timed out",
        ),
    ] {
        let profile = fake_profile(&scoped.path, name, ending, true)?;
        let path = profile.cli_path.as_ref().unwrap();
        let script = std::fs::read_to_string(path)?.replace("cat >/dev/null", input_action);
        std::fs::write(path, script)?;
        // Warm the final rewritten script before starting the behavior deadline.
        assert!(std::process::Command::new(path)
            .arg("--remem-fixture-ready")
            .status()?
            .success());
        let started = std::time::Instant::now();
        let error = runtime
            .block_on(super::codex_cli::call_codex_cli_with_timeout(
                "test",
                &prompt,
                &profile,
                std::time::Duration::from_millis(250),
                None,
            ))
            .unwrap_err();
        assert!(started.elapsed() < std::time::Duration::from_secs(3));
        assert!(error.to_string().contains(message), "{error}");
        let evidence = &error
            .downcast_ref::<super::types::AiCallFailure>()
            .unwrap()
            .evidence;
        assert_eq!(evidence.usage.as_ref().unwrap().known_total_tokens, 140);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn usage_attempt_outer_cancellation_records_observed_or_missing_once() -> Result<()> {
    let scope = crate::db::test_support::ScopedTestDataDir::new("usage-outer-cancel");
    // Complete lazy migrations before measuring cancellation of the fake CLI.
    drop(crate::db::open_db()?);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let ctx = super::UsageContext {
        project: Some("/fixture"),
        session_id: None,
        operation: "outer-cancel",
        host: None,
        profile: None,
    };
    for (name, observed) in [("reported", true), ("no-newline", true), ("missing", false)] {
        let profile = fake_profile(&scope.path, name, "while :; do sleep 0.02; done", observed)?;
        if name == "no-newline" {
            let path = profile.cli_path.as_ref().unwrap();
            let script = std::fs::read_to_string(path)?.replace("printf '%s\\n'", "printf '%s'");
            std::fs::write(path, script)?;
        }
        let outer = runtime.block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_millis(750),
                super::with_resolved_profile(profile, super::call_ai("test", "fixture", ctx)),
            )
            .await
        });
        assert!(outer.is_err());
    }
    let profile = fake_profile(&scope.path, "success", "printf ok > \"$output\"", true)?;
    assert_eq!(
        runtime.block_on(super::with_resolved_profile(
            profile,
            super::call_ai("test", "fixture", ctx)
        ))?,
        "ok"
    );
    let conn = crate::db::open_db()?;
    let rows: Vec<(String, String, i64)> = conn
        .prepare(
            "SELECT attempt_outcome, usage_status, total_tokens FROM ai_usage_events ORDER BY id",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    assert_eq!(
        rows,
        vec![
            ("failed".into(), "complete".into(), 140),
            ("failed".into(), "complete".into(), 140),
            ("failed".into(), "missing".into(), 0),
            ("success".into(), "complete".into(), 140)
        ]
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn usage_attempt_auto_model_is_unpriced_without_global_override() -> Result<()> {
    let dir = std::env::temp_dir().join(format!(
        "remem-usage-auto-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir)?;
    let mut profile = fake_profile(&dir, "auto", "printf ok > \"$output\"", true)?;
    profile.model = None;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    for (config, expected_status) in [
        ("version = 1\n", "unpriced"),
        (
            "[pricing]\ninput_per_mtok = 2.0\noutput_per_mtok = 8.0\n",
            "complete",
        ),
    ] {
        super::tests::with_pricing_config(
            config,
            &[
                ("REMEM_PRICE_INPUT_PER_MTOK", None),
                ("REMEM_PRICE_OUTPUT_PER_MTOK", None),
            ],
            || -> Result<()> {
                let evidence = crate::db::with_data_dir(&dir, || {
                    runtime.block_on(super::codex_cli::call_codex_cli_with_timeout(
                        "test",
                        "fixture",
                        &profile,
                        std::time::Duration::from_secs(2),
                        None,
                    ))
                })?;
                assert_eq!(evidence.model, "codex-default");
                let (cost, source, status) = super::pricing::estimate_observed_cost_usd(
                    &evidence.model,
                    evidence.usage.as_ref().unwrap(),
                )?;
                assert_eq!(status, expected_status);
                if status == "unpriced" {
                    assert_eq!((cost, source), (0.0, "unknown_pricing"));
                } else {
                    assert!((cost - 0.00052).abs() < 1e-12);
                    assert_eq!(source, "config_override");
                }
                Ok(())
            },
        )?;
    }
    std::fs::remove_dir_all(dir)?;
    Ok(())
}

#[test]
fn usage_attempt_guard_frames_split_turns_and_ignores_partial_json() -> Result<()> {
    let _scope = crate::db::test_support::ScopedTestDataDir::new("usage-framing");
    let profile = crate::runtime_config::ResolvedMemoryAiProfile {
        profile_name: "fixture".into(),
        executor: crate::runtime_config::MemoryAiExecutor::CodexCli,
        model: Some("gpt-5.2".into()),
        cli_path: None,
        base_url: None,
        reasoning_effort: None,
    };
    let ctx = super::UsageContext {
        project: None,
        session_id: None,
        operation: "framing",
        host: None,
        profile: None,
    };
    let first = json!({"type":"turn.completed","usage":{"input_tokens":100,"cached_input_tokens":0,"output_tokens":40,"reasoning_output_tokens":0}}).to_string();
    let second = json!({"type":"turn.completed","usage":{"input_tokens":50,"cached_input_tokens":0,"output_tokens":10,"reasoning_output_tokens":0}}).to_string();
    let mut guard = super::usage::UsageAttemptGuard::new(ctx, &profile);
    for bytes in [
        &first.as_bytes()[..20],
        &first.as_bytes()[20..],
        b"\n",
        second.as_bytes(),
    ] {
        guard.observe_codex_bytes(bytes);
    }
    let snapshot = guard.snapshot();
    assert_eq!(snapshot.usage.as_ref().unwrap().known_total_tokens, 200);
    assert_eq!(
        guard.snapshot().usage.as_ref().unwrap().known_total_tokens,
        200,
        "snapshots must not merge trailing events twice"
    );
    guard.complete(&snapshot, "success", 0, 0);
    drop(guard);
    let incomplete = super::usage::UsageAttemptGuard::new(ctx, &profile);
    incomplete.observe_codex_bytes(&first.as_bytes()[..20]);
    drop(incomplete);
    let conn = crate::db::open_db()?;
    let totals: (i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*),SUM(total_tokens),SUM(usage_status='missing') FROM ai_usage_events",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(totals, (2, 200, 1));
    Ok(())
}
