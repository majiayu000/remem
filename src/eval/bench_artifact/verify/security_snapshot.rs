use anyhow::{ensure, Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{resolve_public_path, VerifyState};
use crate::eval::bench_artifact::{MemoryRunArtifact, VerifiedArtifact};
use crate::eval::memory_bench::types::{MemoryBenchSuiteFixture, MemoryBenchTask};

pub(super) fn validate_security_snapshot(
    run: &MemoryRunArtifact,
    label: &str,
    state: &mut VerifyState,
) {
    let Some(raw_path) = run.artifacts.get("remem_db_snapshot") else {
        return;
    };
    let Some(path) = resolve_public_path(state, raw_path, raw_path) else {
        return;
    };
    if path.extension().and_then(|value| value.to_str()) != Some("sqlite3") {
        state.fail(
            raw_path.clone(),
            "security snapshot must be an explicit SQLite file",
        );
        return;
    }
    let connection = match Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(error) => {
            state.fail(
                raw_path.clone(),
                format!("open security SQLite snapshot: {error}"),
            );
            return;
        }
    };
    if connection
        .query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
        .as_deref()
        != Ok("ok")
    {
        state.fail(
            raw_path.clone(),
            "security SQLite snapshot failed quick_check",
        );
        return;
    }
    if let Err(error) = validate_semantics(&connection, run, state) {
        state.fail(
            raw_path.clone(),
            format!("snapshot semantic contract mismatch: {error:#}"),
        );
    }
    if run
        .suite_content_identity
        .as_deref()
        .is_none_or(str::is_empty)
    {
        state.fail(
            label.to_string(),
            "v2 memory run lacks suite_content_identity",
        );
    }
}

fn validate_semantics(
    connection: &Connection,
    run: &MemoryRunArtifact,
    state: &mut VerifyState,
) -> Result<()> {
    let suite_artifact = verified_security_suite(state)?;
    let suite = &suite_artifact.value;
    let suite_identity = format!("sha256-raw-suite-v1:{}", suite_artifact.sha256);
    ensure!(
        run.suite_content_identity.as_deref() == Some(suite_identity.as_str()),
        "run suite identity does not match verifier-consumed bytes"
    );
    ensure!(
        suite.benchmark_id == run.benchmark_id
            && suite.version == run.benchmark_version
            && suite.suite == run.suite
            && run.environment.fixture_revision.as_deref() == Some(suite.fixture_revision.as_str()),
        "run metadata does not match typed suite"
    );
    let task = suite
        .tasks
        .iter()
        .find(|task| task.id == run.task_id)
        .context("run task is absent from typed suite")?;
    ensure!(
        run.reference_time_epoch == task.reference_time_epoch
            && run.retrieval.gold_supporting_event_ids == task.gold_supporting_event_ids,
        "run task fields do not match typed suite"
    );
    validate_run_policy_contract(run, task)?;

    let expected = expected_events(task)?;
    let actual = read_events(connection, &run.task_id)?;
    ensure!(
        expected == actual,
        "event identity differs: expected={} actual={}",
        semantic_identity(&expected)?,
        semantic_identity(&actual)?
    );
    validate_policy_state(connection, task)
}

fn verified_security_suite(
    state: &mut VerifyState,
) -> Result<VerifiedArtifact<MemoryBenchSuiteFixture>> {
    const RELATIVE: &str = "memory/suites/adversarial-policy/suite.json";
    if let Some(suite) = state
        .verified_artifacts
        .memory_suites
        .iter()
        .find(|suite| suite.path == RELATIVE)
    {
        return Ok(suite.clone());
    }
    let path = state.root.join(RELATIVE);
    let bytes = std::fs::read(&path)
        .with_context(|| format!("read typed security suite {}", path.display()))?;
    let artifact = VerifiedArtifact {
        path: RELATIVE.to_string(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        value: serde_json::from_slice(&bytes)
            .with_context(|| format!("parse typed security suite {}", path.display()))?,
    };
    state
        .verified_artifacts
        .memory_suites
        .push(artifact.clone());
    Ok(artifact)
}

fn validate_run_policy_contract(run: &MemoryRunArtifact, task: &MemoryBenchTask) -> Result<()> {
    let Some(policy) = task.policy.as_ref() else {
        return Ok(());
    };
    ensure!(
        run.diagnosis.policy_abstention == policy.expected_policy_abstention,
        "run policy abstention differs from suite expectation"
    );
    for (pointer, expected) in [
        (
            "/policy/active_claim_count",
            u64::from(policy.expected_active_claims),
        ),
        (
            "/policy/candidate_count",
            u64::from(policy.expected_candidates),
        ),
        (
            "/policy/summary_input_count",
            u64::from(policy.expected_summary_inputs),
        ),
    ] {
        ensure!(
            run.metrics
                .pointer(pointer)
                .and_then(serde_json::Value::as_u64)
                == Some(expected),
            "run metric {pointer} differs from suite expectation"
        );
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct SnapshotEventSemantic {
    event_id: String,
    event_type: String,
    role: Option<String>,
    tool_name: Option<String>,
    content: String,
    content_hash: String,
    retention_class: String,
    created_at_epoch: i64,
    reference_time_epoch: Option<i64>,
    host: String,
    project: String,
}

fn expected_events(task: &MemoryBenchTask) -> Result<Vec<SnapshotEventSemantic>> {
    let explicitly_approved = task
        .policy
        .as_ref()
        .is_some_and(|policy| policy.explicit_approval);
    if task
        .policy
        .as_ref()
        .is_some_and(|policy| policy.sensitive_or_restricted && !policy.explicit_approval)
    {
        ensure!(
            task.evidence
                .iter()
                .all(|evidence| !evidence.retention_allowed),
            "restricted suite evidence must be non-retainable"
        );
    }
    let mut events = task
        .evidence
        .iter()
        .map(|evidence| {
            let created_at_epoch = evidence.created_at_epoch.with_context(|| {
                format!("evidence {} lacks created_at_epoch", evidence.event_id)
            })?;
            let content = crate::db::capture::redact_capture_content(&evidence.content);
            Ok(SnapshotEventSemantic {
                event_id: evidence.event_id.clone(),
                event_type: if explicitly_approved {
                    "user_prompt_submit".to_string()
                } else {
                    "tool_result".to_string()
                },
                role: explicitly_approved.then(|| "user".to_string()),
                tool_name: (!explicitly_approved).then(|| "Bash".to_string()),
                content_hash: crate::db::content_identity_hash(content.as_bytes()),
                content,
                retention_class: "raw_keep".to_string(),
                created_at_epoch,
                reference_time_epoch: Some(created_at_epoch),
                host: "codex-cli".to_string(),
                project: "/tmp/remem-memory-bench/repo".to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    events.sort_by(|left, right| left.event_id.cmp(&right.event_id));
    Ok(events)
}

fn read_events(connection: &Connection, task_id: &str) -> Result<Vec<SnapshotEventSemantic>> {
    let mut statement = connection.prepare(
        "SELECT e.event_id, e.event_type, e.role, e.tool_name,
                COALESCE(CASE WHEN b.content_encoding = 'plain'
                              THEN CAST(b.content_bytes AS TEXT)
                              ELSE NULL END, e.content_text, ''),
                e.content_hash, e.retention_class, e.created_at_epoch,
                e.reference_time_epoch, h.name, p.project_path, e.content_blob_id
         FROM captured_events e
         JOIN hosts h ON h.id = e.host_id
         JOIN projects p ON p.id = e.project_id
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         WHERE e.session_id = ?1
         ORDER BY e.event_id",
    )?;
    let rows = statement.query_map([task_id], |row| {
        Ok((
            SnapshotEventSemantic {
                event_id: row.get(0)?,
                event_type: row.get(1)?,
                role: row.get(2)?,
                tool_name: row.get(3)?,
                content: row.get(4)?,
                content_hash: row.get(5)?,
                retention_class: row.get(6)?,
                created_at_epoch: row.get(7)?,
                reference_time_epoch: row.get(8)?,
                host: row.get(9)?,
                project: row.get(10)?,
            },
            row.get::<_, Option<i64>>(11)?,
        ))
    })?;
    rows.map(|row| {
        let (event, blob_id) = row?;
        ensure!(
            blob_id.is_none(),
            "security source event must use exact raw_keep storage"
        );
        Ok(event)
    })
    .collect()
}

fn semantic_identity(events: &[SnapshotEventSemantic]) -> Result<String> {
    Ok(format!(
        "sha256-snapshot-semantics-v1:{:x}",
        Sha256::digest(serde_json::to_vec(events)?)
    ))
}

fn validate_policy_state(connection: &Connection, task: &MemoryBenchTask) -> Result<()> {
    let Some(policy) = task.policy.as_ref() else {
        return Ok(());
    };
    let active_claims: i64 = connection.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    let candidates: i64 = connection.query_row(
        "SELECT COUNT(*) FROM memory_candidates
         WHERE review_status IN ('pending_review', 'quarantined', 'deferred')",
        [],
        |row| row.get(0),
    )?;
    let mut statement =
        connection.prepare("SELECT content FROM memories WHERE status = 'active'")?;
    let texts = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let summary_inputs = texts
        .iter()
        .filter(|text| crate::memory::poisoning::scan_instruction_pattern(text).is_none())
        .count();
    ensure!(
        active_claims == i64::from(policy.expected_active_claims)
            && candidates == i64::from(policy.expected_candidates)
            && summary_inputs == policy.expected_summary_inputs as usize,
        "governance counts differ from suite expectations"
    );
    let expected_trust = if policy.explicit_approval {
        "user_prompt"
    } else {
        "local_tool_output"
    };
    let invalid_trust: i64 = connection.query_row(
        "SELECT
           (SELECT COUNT(*) FROM memories WHERE source_trust_class != ?1) +
           (SELECT COUNT(*) FROM memory_candidates WHERE source_trust_class != ?1)",
        [expected_trust],
        |row| row.get(0),
    )?;
    ensure!(
        invalid_trust == 0,
        "trust classification differs from suite source contract"
    );
    Ok(())
}
