use std::collections::BTreeMap;
#[cfg(test)]
use std::{cell::RefCell, path::Path};
use std::{ptr, ptr::NonNull};

use anyhow::{ensure, Context, Result};
use rusqlite::serialize::OwnedData;
use rusqlite::{ffi, Connection, DatabaseName};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{resolve_public_path, VerifyState};
use crate::eval::bench_artifact::{MemoryRunArtifact, VerifiedArtifact};
use crate::eval::memory_bench::types::{
    MemoryBenchCondition, MemoryBenchPolicyOutcome, MemoryBenchSuiteFixture, MemoryBenchTask,
};

mod inventory;
#[cfg(test)]
mod tests;

#[cfg(test)]
thread_local! {
    static AFTER_SNAPSHOT_CONSUMED: RefCell<Option<Box<dyn FnOnce(&Path)>>> = RefCell::new(None);
}

#[cfg(test)]
pub(in crate::eval::bench_artifact) fn set_after_security_snapshot_consumed_hook(
    hook: impl FnOnce(&Path) + 'static,
) {
    AFTER_SNAPSHOT_CONSUMED.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "snapshot consumption hook already set"
        );
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_after_security_snapshot_consumed_hook(path: &Path) {
    AFTER_SNAPSHOT_CONSUMED.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(path);
        }
    });
}

#[cfg(not(test))]
fn run_after_security_snapshot_consumed_hook(_path: &std::path::Path) {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TrustedSnapshotCacheKey {
    suite_content_sha256: String,
    task_semantic_sha256: String,
    production_input_pathspec_sha256: String,
    artifact_os: String,
    artifact_arch: String,
}

impl TrustedSnapshotCacheKey {
    fn new(
        suite_content_sha256: &str,
        task: &MemoryBenchTask,
        artifact_os: &str,
        artifact_arch: &str,
    ) -> Result<Self> {
        let task_bytes = serde_json::to_vec(task).context("serialize typed security task")?;
        Ok(Self {
            suite_content_sha256: suite_content_sha256.to_string(),
            task_semantic_sha256: format!("{:x}", Sha256::digest(task_bytes)),
            production_input_pathspec_sha256: production_input_pathspec_sha256(),
            artifact_os: artifact_os.to_string(),
            artifact_arch: artifact_arch.to_string(),
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct VerificationContext {
    trusted_security_snapshots: BTreeMap<
        TrustedSnapshotCacheKey,
        crate::eval::security_snapshot_identity::SnapshotIdentity,
    >,
}

impl VerificationContext {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn trusted_snapshot_identity(
        &mut self,
        suite_content_sha256: &str,
        task: &MemoryBenchTask,
        artifact_os: &str,
        artifact_arch: &str,
        replay: impl FnOnce(
            &MemoryBenchTask,
        )
            -> Result<crate::eval::security_snapshot_identity::SnapshotIdentity>,
    ) -> Result<crate::eval::security_snapshot_identity::SnapshotIdentity> {
        let key =
            TrustedSnapshotCacheKey::new(suite_content_sha256, task, artifact_os, artifact_arch)?;
        if let Some(identity) = self.trusted_security_snapshots.get(&key) {
            return Ok(identity.clone());
        }
        let identity = replay(task)?;
        self.trusted_security_snapshots
            .insert(key, identity.clone());
        Ok(identity)
    }
}

fn production_input_pathspec_sha256() -> String {
    format!(
        "{:x}",
        Sha256::digest(include_bytes!(
            "../../../../eval/production-input-pathspec-v1.json"
        ))
    )
}

pub(super) fn validate_security_snapshot(
    run: &MemoryRunArtifact,
    label: &str,
    state: &mut VerifyState,
    context: &mut VerificationContext,
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
    let snapshot_bytes = match state.consume_file(&path, "read security SQLite snapshot") {
        Ok(bytes) => bytes,
        Err(()) => return,
    };
    run_after_security_snapshot_consumed_hook(&path);
    let connection = match open_consumed_read_only_sqlite(&snapshot_bytes) {
        Ok(connection) => connection,
        Err(error) => {
            state.fail(
                raw_path.clone(),
                format!("open consumed security SQLite snapshot: {error:#}"),
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
    match validate_semantics(&connection, run, state, context) {
        Ok(outcome) => {
            state
                .verified_artifacts
                .security_policy_outcomes
                .insert(label.to_string(), outcome);
        }
        Err(error) => {
            state.fail(
                raw_path.clone(),
                format!("snapshot semantic contract mismatch: {error:#}"),
            );
        }
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

pub(super) fn open_consumed_read_only_sqlite(bytes: &[u8]) -> Result<Connection> {
    ensure!(!bytes.is_empty(), "SQLite snapshot is empty");
    let allocation_size = u64::try_from(bytes.len()).context("SQLite snapshot is too large")?;
    // SAFETY: sqlite3_malloc64 returns either null or an allocation owned by SQLite. We copy
    // exactly `bytes.len()` initialized bytes into that allocation, create no aliases to it, and
    // immediately transfer exclusive ownership to OwnedData, whose Drop uses sqlite3_free.
    let owned = unsafe {
        let allocation = ffi::sqlite3_malloc64(allocation_size);
        let allocation =
            NonNull::new(allocation.cast::<u8>()).context("allocate consumed SQLite snapshot")?;
        ptr::copy_nonoverlapping(bytes.as_ptr(), allocation.as_ptr(), bytes.len());
        OwnedData::from_raw_nonnull(allocation, bytes.len())
    };
    let mut connection = Connection::open_in_memory().context("open in-memory SQLite handle")?;
    connection
        .deserialize(DatabaseName::Main, owned, true)
        .context("deserialize consumed SQLite snapshot as read-only")?;
    Ok(connection)
}

fn validate_semantics(
    connection: &Connection,
    run: &MemoryRunArtifact,
    state: &mut VerifyState,
    context: &mut VerificationContext,
) -> Result<MemoryBenchPolicyOutcome> {
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
    inventory::validate_closed_world(connection, task, expected.len())?;
    validate_full_snapshot_identity(
        connection,
        task,
        &suite_artifact.sha256,
        &run.environment.os,
        &run.environment.arch,
        context,
    )?;
    validate_policy_state(connection, task)?;
    recompute_policy_outcome(connection, run, task, state)
}

fn validate_full_snapshot_identity(
    connection: &Connection,
    task: &MemoryBenchTask,
    suite_content_sha256: &str,
    artifact_os: &str,
    artifact_arch: &str,
    context: &mut VerificationContext,
) -> Result<()> {
    let actual = crate::eval::security_snapshot_identity::snapshot_identity(connection)?;
    let expected = context.trusted_snapshot_identity(
        suite_content_sha256,
        task,
        artifact_os,
        artifact_arch,
        crate::eval::memory_bench::replay_trusted_security_snapshot_identity,
    )?;
    ensure!(
        actual == expected,
        "complete typed snapshot identity differs: {}",
        snapshot_identity_delta(&expected, &actual)
    );
    Ok(())
}

fn snapshot_identity_delta(
    expected: &crate::eval::security_snapshot_identity::SnapshotIdentity,
    actual: &crate::eval::security_snapshot_identity::SnapshotIdentity,
) -> String {
    expected
        .keys()
        .chain(actual.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter(|key| expected.get(*key) != actual.get(*key))
        .map(|key| key.as_str())
        .collect::<Vec<_>>()
        .join(",")
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
    let bytes = state
        .consume_file(&path, "read typed security suite")
        .map_err(|()| anyhow::anyhow!("typed security suite bytes are unavailable"))?;
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

fn recompute_policy_outcome(
    connection: &Connection,
    run: &MemoryRunArtifact,
    task: &MemoryBenchTask,
    state: &mut VerifyState,
) -> Result<MemoryBenchPolicyOutcome> {
    let answer = referenced_json(run, "answer", state)?;
    let abstained = answer
        .get("abstained")
        .and_then(Value::as_bool)
        .context("referenced answer lacks boolean abstained")?;
    ensure!(
        run.answer.get("abstained").and_then(Value::as_bool) == Some(abstained),
        "typed run answer differs from referenced answer evidence"
    );
    let retrieval = referenced_json(run, "retrieved_evidence", state)?;
    let retrieved_event_ids = retrieval
        .get("retrieved")
        .and_then(Value::as_array)
        .context("referenced retrieval evidence lacks retrieved array")?
        .iter()
        .map(|item| {
            item.get("event_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .context("referenced retrieval item lacks event_id")
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        retrieved_event_ids == run.retrieval.retrieved_supporting_evidence_ids,
        "typed run retrieval differs from referenced retrieval evidence"
    );

    let active_claim_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    let candidate_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM memory_candidates
         WHERE review_status IN ('pending_review', 'quarantined', 'deferred')",
        [],
        |row| row.get(0),
    )?;
    let mut statement =
        connection.prepare("SELECT content FROM memories WHERE status = 'active'")?;
    let active_texts = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let summary_input_count = active_texts
        .iter()
        .filter(|text| crate::memory::poisoning::scan_instruction_pattern(text).is_none())
        .count();
    let quarantined_observations: i64 = connection.query_row(
        "SELECT COUNT(*) FROM observations WHERE status = 'poisoning_quarantined'",
        [],
        |row| row.get(0),
    )?;
    let quarantined_candidates: i64 = connection.query_row(
        "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'quarantined'",
        [],
        |row| row.get(0),
    )?;
    let condition = MemoryBenchCondition::parse(&run.condition)
        .with_context(|| format!("unsupported security condition {}", run.condition))?;
    let policy_state = crate::eval::memory_bench::VerifiedSecurityPolicyState {
        active_claim_count: u32::try_from(active_claim_count)
            .context("active claim count does not fit u32")?,
        candidate_count: u32::try_from(candidate_count)
            .context("candidate count does not fit u32")?,
        summary_input_count: u32::try_from(summary_input_count)
            .context("summary input count does not fit u32")?,
        poisoning_source_scanner_matched: scan_persisted_source_events(connection)?,
        poisoning_generated_surface_blocked: quarantined_observations > 0
            || quarantined_candidates > 0,
    };
    Ok(crate::eval::memory_bench::score_verified_security_policy(
        condition,
        task,
        &retrieved_event_ids,
        abstained,
        policy_state,
    ))
}

fn referenced_json(run: &MemoryRunArtifact, key: &str, state: &mut VerifyState) -> Result<Value> {
    let raw_path = run
        .artifacts
        .get(key)
        .with_context(|| format!("security run lacks {key} artifact"))?;
    let path = resolve_public_path(state, raw_path, raw_path)
        .with_context(|| format!("security {key} artifact path is invalid"))?;
    let bytes = state
        .consume_file(&path, &format!("read security {key} artifact"))
        .map_err(|()| anyhow::anyhow!("security {key} artifact bytes are unavailable"))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse security {key} artifact"))
}

fn scan_persisted_source_events(connection: &Connection) -> Result<bool> {
    let mut statement = connection.prepare(
        "SELECT e.id,
                COALESCE(
                    CASE WHEN b.content_encoding = 'plain'
                         THEN CAST(b.content_bytes AS TEXT)
                         ELSE NULL END,
                    e.content_text,
                    '')
         FROM captured_events e
         LEFT JOIN event_blobs b ON b.id = e.content_blob_id
         ORDER BY e.id ASC",
    )?;
    let events = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(crate::memory::poisoning::scan_source_events(
        events
            .iter()
            .map(|(event_id, content)| (*event_id, content.as_str())),
    )
    .is_some())
}
