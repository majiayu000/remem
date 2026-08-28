use anyhow::{Context, Result};
use rusqlite::{backup::StepResult, Connection, DatabaseName};
use serde_json::json;
use std::sync::{Mutex, OnceLock};

use super::runner::{RetrievedEvidence, PROJECT};
use super::types::{MemoryBenchEvidence, MemoryBenchPolicyMeasurement, MemoryBenchTask};

const LEASE_OWNER: &str = "memory-bench-production-pipeline";

pub(super) async fn retrieve_with_production_pipeline(
    task: &MemoryBenchTask,
) -> Result<(
    Vec<RetrievedEvidence>,
    MemoryBenchPolicyMeasurement,
    Vec<u8>,
)> {
    let (conn, retrieved, measurement) = execute_production_pipeline(task).await?;
    let snapshot = conn.serialize(DatabaseName::Main)?.to_vec();
    Ok((retrieved, measurement, snapshot))
}

pub(super) async fn trusted_snapshot_identity(
    task: &MemoryBenchTask,
) -> Result<crate::eval::security_snapshot_identity::SnapshotIdentity> {
    let conn = trusted_schema_connection()?;
    let (conn, _, _) = execute_production_pipeline_with_connection(conn, task).await?;
    crate::eval::security_snapshot_identity::snapshot_identity(&conn)
}

async fn execute_production_pipeline(
    task: &MemoryBenchTask,
) -> Result<(
    Connection,
    Vec<RetrievedEvidence>,
    MemoryBenchPolicyMeasurement,
)> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    execute_production_pipeline_with_connection(conn, task).await
}

async fn execute_production_pipeline_with_connection(
    mut conn: Connection,
    task: &MemoryBenchTask,
) -> Result<(
    Connection,
    Vec<RetrievedEvidence>,
    MemoryBenchPolicyMeasurement,
)> {
    let policy = task.policy.as_ref();
    let poisoning_expected = policy.is_some_and(|policy| policy.poisoning_quarantine_expected);
    let explicitly_approved = policy.is_some_and(|policy| policy.explicit_approval);

    record_fixture_events(&conn, task, explicitly_approved)?;
    run_observation_and_candidate_tasks(&mut conn, task, poisoning_expected).await?;

    let active_memories =
        crate::memory::list_memories(&conn, PROJECT, None, i64::MAX, 0, false, Some("main"))?;
    let reviewable_candidates: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_candidates
         WHERE review_status IN ('pending_review', 'quarantined', 'deferred')",
        [],
        |row| row.get(0),
    )?;
    let quarantined_observations: i64 = conn.query_row(
        "SELECT COUNT(*) FROM observations WHERE status = 'poisoning_quarantined'",
        [],
        |row| row.get(0),
    )?;
    let quarantined_candidates: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'quarantined'",
        [],
        |row| row.get(0),
    )?;
    let summary_input_count = active_memories
        .iter()
        .filter(|memory| crate::memory::poisoning::scan_instruction_pattern(&memory.text).is_none())
        .count();
    let source_scanner_matched = scan_persisted_source_events(&conn)?;
    let retrieved = search_and_map_evidence(&conn, task)?;

    let measurement = MemoryBenchPolicyMeasurement {
        verification_path: "capture_observation_candidate_promotion".to_string(),
        measurement_source: "sqlite_production_tables".to_string(),
        source_scanner_config:
            "scan_source_instruction_pattern(include_opaque_payload=false); generated_surfaces=scan_instruction_pattern(include_opaque_payload=true)"
                .to_string(),
        active_claim_count: u32::try_from(active_memories.len())?,
        candidate_count: u32::try_from(reviewable_candidates)?,
        summary_input_count: u32::try_from(summary_input_count)?,
        poisoning_source_scanner_matched: source_scanner_matched,
        poisoning_generated_surface_blocked: quarantined_observations > 0
            || quarantined_candidates > 0,
    };
    Ok((conn, retrieved, measurement))
}

fn trusted_schema_connection() -> Result<Connection> {
    static SCHEMA: OnceLock<Result<Mutex<Connection>, String>> = OnceLock::new();
    let schema = SCHEMA
        .get_or_init(|| {
            let connection = Connection::open_in_memory().map_err(|error| error.to_string())?;
            crate::migrate::run_migrations(&connection).map_err(|error| format!("{error:#}"))?;
            Ok(Mutex::new(connection))
        })
        .as_ref()
        .map_err(|error| anyhow::anyhow!("initialize trusted schema cache: {error}"))?;
    let schema = schema
        .lock()
        .map_err(|_| anyhow::anyhow!("trusted schema cache is poisoned"))?;
    let mut connection = Connection::open_in_memory()?;
    let backup = rusqlite::backup::Backup::new(&schema, &mut connection)?;
    anyhow::ensure!(
        backup.step(-1)? == StepResult::Done,
        "trusted schema cache backup did not complete"
    );
    drop(backup);
    Ok(connection)
}

fn record_fixture_events(
    conn: &Connection,
    task: &MemoryBenchTask,
    explicitly_approved: bool,
) -> Result<()> {
    for evidence in &task.evidence {
        crate::db::record_captured_event_with_id_and_reference_time(
            conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id: &task.id,
                project: PROJECT,
                cwd: None,
                event_type: if explicitly_approved {
                    "user_prompt_submit"
                } else {
                    "tool_result"
                },
                role: explicitly_approved.then_some("user"),
                tool_name: (!explicitly_approved).then_some("Bash"),
                content: &evidence.content,
                task_kind: Some(crate::db::ExtractionTaskKind::ObservationExtract),
            },
            Some(&evidence.event_id),
            evidence.created_at_epoch,
        )?;
    }
    Ok(())
}

async fn run_observation_and_candidate_tasks(
    conn: &mut Connection,
    task: &MemoryBenchTask,
    poisoning_expected: bool,
) -> Result<()> {
    let observation_task = crate::db::claim_next_extraction_task(conn, LEASE_OWNER, 60)?
        .context("production-path benchmark expected observation extraction task")?;
    anyhow::ensure!(
        observation_task.task_kind == crate::db::ExtractionTaskKind::ObservationExtract,
        "production-path benchmark claimed unexpected task kind {}",
        observation_task.task_kind.as_str()
    );
    let extraction_response = deterministic_observation_response(task, poisoning_expected);
    let observation_result = crate::observation_extract::process_with_extractor(
        conn,
        &observation_task,
        |_prompt| async move { Ok(extraction_response) },
    )
    .await?;
    crate::db::mark_extraction_task_done(
        conn,
        observation_task.id,
        LEASE_OWNER,
        observation_task.high_watermark_event_id,
    )?;

    if !matches!(
        observation_result,
        crate::observation_extract::ObservationExtractResult::Written(_)
    ) {
        return Ok(());
    }
    let candidate_task = crate::db::claim_next_extraction_task(conn, LEASE_OWNER, 60)?
        .context("production-path benchmark expected memory candidate task")?;
    anyhow::ensure!(
        candidate_task.task_kind == crate::db::ExtractionTaskKind::MemoryCandidate,
        "production-path benchmark claimed unexpected follow-up task kind {}",
        candidate_task.task_kind.as_str()
    );
    let candidate_response = deterministic_candidate_response(task);
    let candidate_result = crate::memory_candidate::process_with_generator(
        conn,
        &candidate_task,
        |_prompt| async move { Ok(candidate_response) },
    )
    .await?;
    let completed_to_event_id = match candidate_result {
        crate::memory_candidate::MemoryCandidateResult::Written { to_event_id, .. } => {
            Some(to_event_id)
        }
        _ => candidate_task.high_watermark_event_id,
    };
    crate::db::mark_extraction_task_done(
        conn,
        candidate_task.id,
        LEASE_OWNER,
        completed_to_event_id,
    )?;
    Ok(())
}

fn scan_persisted_source_events(conn: &Connection) -> Result<bool> {
    let persisted_source_events = {
        let mut stmt = conn.prepare(
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
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    Ok(crate::memory::poisoning::scan_source_events(
        persisted_source_events
            .iter()
            .map(|(event_id, content)| (*event_id, content.as_str())),
    )
    .is_some())
}

fn search_and_map_evidence(
    conn: &Connection,
    task: &MemoryBenchTask,
) -> Result<Vec<RetrievedEvidence>> {
    let hits = crate::retrieval::search::search_with_branch(
        conn,
        Some(&task.query),
        Some(PROJECT),
        None,
        5,
        0,
        false,
        Some("main"),
    )?;
    Ok(hits
        .into_iter()
        .filter_map(|memory| {
            task.evidence
                .iter()
                .find(|evidence| {
                    evidence.retention_allowed
                        && evidence.topic_key.as_deref() == memory.topic_key.as_deref()
                })
                .map(|evidence| RetrievedEvidence::from_memory(memory.id, evidence))
        })
        .collect())
}

fn deterministic_observation_response(task: &MemoryBenchTask, poisoning_expected: bool) -> String {
    let observations = task
        .evidence
        .iter()
        .filter(|evidence| evidence.retention_allowed || poisoning_expected)
        .map(|evidence| {
            let observation_type = match evidence.memory_type.as_str() {
                "bugfix" | "decision" | "discovery" => evidence.memory_type.as_str(),
                "architecture" | "procedure" => "discovery",
                "lesson" => "bugfix",
                _ => "decision",
            };
            let narrative = deterministic_evidence_claim(evidence, explicitly_approved(task));
            json!({
                "type": observation_type,
                "title": evidence.title,
                "subtitle": null,
                "narrative": narrative,
                "facts": [],
                "concepts": [],
                "files_read": evidence.files,
                "files_modified": [],
                "confidence": 0.99,
            })
        })
        .collect::<Vec<_>>();
    if observations.is_empty() {
        json!({"no_observations":{"reason":"fixture policy rejects durable retention"}}).to_string()
    } else {
        json!({"observations": observations}).to_string()
    }
}

fn deterministic_candidate_response(task: &MemoryBenchTask) -> String {
    let explicitly_approved = explicitly_approved(task);
    task.evidence
        .iter()
        .filter(|evidence| evidence.retention_allowed)
        .map(|evidence| {
            let candidate_text = deterministic_evidence_claim(evidence, explicitly_approved);
            format!(
                "<memory_candidate><scope>project</scope><type>{}</type><topic_key>{}</topic_key><risk_class>low</risk_class><confidence>0.99</confidence><text>{}</text></memory_candidate>",
                crate::memory::format::xml_escape_text(&evidence.memory_type),
                crate::memory::format::xml_escape_text(evidence.topic_key.as_deref().unwrap_or("benchmark-evidence")),
                crate::memory::format::xml_escape_text(&candidate_text),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn explicitly_approved(task: &MemoryBenchTask) -> bool {
    task.policy
        .as_ref()
        .is_some_and(|policy| policy.explicit_approval)
}

fn deterministic_evidence_claim(
    evidence: &MemoryBenchEvidence,
    explicitly_approved: bool,
) -> String {
    if explicitly_approved {
        let value = evidence
            .content
            .rsplit_once(" is ")
            .map_or(evidence.content.as_str(), |(_, value)| value);
        format!(
            "The user approved this external source schema endpoint, {}, to retain as memory.",
            value.trim_end_matches('.')
        )
    } else {
        evidence.content.clone()
    }
}
