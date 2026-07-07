use std::collections::BTreeMap;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

mod evidence;
mod export;
mod list;
mod trace_store;

pub use list::{list_promoted_procedures, ProcedureListItem};

#[cfg(test)]
mod incremental_tests;

const DEFAULT_MIN_VERIFIED_RUNS: usize = 2;
const DEFAULT_MAX_VERIFICATION_AGE_SECS: i64 = 14 * 24 * 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcedureTrace {
    pub project: String,
    pub branch: Option<String>,
    pub workflow_key: String,
    pub command: String,
    pub files_touched: Vec<String>,
    pub succeeded: bool,
    pub verified_at_epoch: i64,
    pub source_event_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcedurePromotionPolicy {
    pub min_verified_runs: usize,
    pub max_verification_age_secs: i64,
}

impl Default for ProcedurePromotionPolicy {
    fn default() -> Self {
        Self {
            min_verified_runs: DEFAULT_MIN_VERIFIED_RUNS,
            max_verification_age_secs: DEFAULT_MAX_VERIFICATION_AGE_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcedureCandidate {
    pub project: String,
    pub branch: Option<String>,
    pub workflow_key: String,
    pub topic_key: String,
    pub title: String,
    pub content: String,
    pub files: Vec<String>,
    pub source_event_ids: Vec<i64>,
    pub verified_runs: usize,
    pub confidence: f64,
    pub verified_at_epoch: i64,
}

pub fn build_procedure_candidate(
    traces: &[ProcedureTrace],
    now_epoch: i64,
    policy: &ProcedurePromotionPolicy,
) -> Option<ProcedureCandidate> {
    let mut verified: Vec<&ProcedureTrace> = traces
        .iter()
        .filter(|trace| trace.succeeded)
        .filter(|trace| trace.source_event_id.is_some())
        .filter(|trace| {
            now_epoch.saturating_sub(trace.verified_at_epoch) <= policy.max_verification_age_secs
        })
        .collect();
    verified.sort_by_key(|trace| trace.verified_at_epoch);
    if verified.len() < policy.min_verified_runs {
        return None;
    }

    let first = verified[0];
    if verified.iter().any(|trace| {
        trace.project != first.project
            || trace.branch != first.branch
            || trace.workflow_key != first.workflow_key
            || trace.command != first.command
    }) {
        return None;
    }

    let mut source_event_ids: Vec<i64> = verified
        .iter()
        .filter_map(|trace| trace.source_event_id)
        .collect();
    source_event_ids.sort_unstable();
    source_event_ids.dedup();
    if source_event_ids.len() < policy.min_verified_runs {
        return None;
    }

    let mut files = verified
        .iter()
        .flat_map(|trace| trace.files_touched.iter().cloned())
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();

    let verified_at_epoch = verified
        .iter()
        .map(|trace| trace.verified_at_epoch)
        .max()
        .unwrap_or(now_epoch);
    let topic_key = procedure_topic_key(first);
    let confidence = confidence_for_verified_runs(source_event_ids.len());
    let content = render_procedure_content(
        first,
        &files,
        &source_event_ids,
        verified.len(),
        verified_at_epoch,
    );

    Some(ProcedureCandidate {
        project: first.project.clone(),
        branch: first.branch.clone(),
        workflow_key: first.workflow_key.clone(),
        title: format!("Procedure: {}", first.workflow_key),
        topic_key,
        content,
        files,
        source_event_ids,
        verified_runs: verified.len(),
        confidence,
        verified_at_epoch,
    })
}

fn confidence_for_verified_runs(verified_runs: usize) -> f64 {
    (0.7 + (verified_runs as f64 * 0.08)).min(0.95)
}

pub fn promote_procedure_memory(conn: &Connection, candidate: &ProcedureCandidate) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    let files_json = (!candidate.files.is_empty())
        .then(|| serde_json::to_string(&candidate.files))
        .transpose()?;
    let source_events_json = serde_json::to_string(&candidate.source_event_ids)?;
    let memory_id = crate::memory::insert_memory_full(
        &tx,
        None,
        &candidate.project,
        Some(&candidate.topic_key),
        &candidate.title,
        &candidate.content,
        "procedure",
        files_json.as_deref(),
        candidate.branch.as_deref(),
        "project",
        Some(candidate.verified_at_epoch),
    )?;
    tx.execute(
        "UPDATE memories
         SET evidence_event_ids = ?1,
             confidence = ?2
         WHERE id = ?3",
        params![source_events_json, candidate.confidence, memory_id],
    )?;
    tx.commit()?;
    Ok(memory_id)
}

pub(crate) fn promote_verified_procedures_for_task(
    conn: &Connection,
    task: &crate::db::ExtractionTask,
    policy: &ProcedurePromotionPolicy,
) -> Result<usize> {
    let now_epoch = chrono::Utc::now().timestamp();
    let traces = trace_store::load_verified_procedure_traces(conn, task, policy, now_epoch)?;
    let mut groups: BTreeMap<(String, Option<String>, String, String), Vec<ProcedureTrace>> =
        BTreeMap::new();
    for trace in traces {
        groups
            .entry((
                trace.project.clone(),
                trace.branch.clone(),
                trace.workflow_key.clone(),
                trace.command.clone(),
            ))
            .or_default()
            .push(trace);
    }

    let mut promoted = 0usize;
    for traces in groups.into_values() {
        let Some(candidate) = build_procedure_candidate(&traces, now_epoch, policy) else {
            continue;
        };
        let existed = procedure_memory_exists(conn, &candidate.project, &candidate.topic_key)?;
        promote_procedure_memory(conn, &candidate)?;
        if !existed {
            promoted += 1;
        }
    }
    Ok(promoted)
}

fn procedure_topic_key(trace: &ProcedureTrace) -> String {
    crate::memory::slugify_for_topic(
        &format!(
            "procedure {} branch {} command {}",
            trace.workflow_key,
            trace.branch.as_deref().unwrap_or("no-branch"),
            trace.command
        ),
        96,
    )
}

fn procedure_memory_exists(conn: &Connection, project: &str, topic_key: &str) -> Result<bool> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM memories
             WHERE project = ?1
               AND topic_key = ?2
               AND scope = 'project'
             LIMIT 1",
            params![project, topic_key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(existing.is_some())
}

fn render_procedure_content(
    trace: &ProcedureTrace,
    files: &[String],
    source_event_ids: &[i64],
    verified_runs: usize,
    verified_at_epoch: i64,
) -> String {
    let files_line = if files.is_empty() {
        "Files: none recorded".to_string()
    } else {
        format!("Files: {}", files.join(", "))
    };
    format!(
        "Procedure: {}\nCommand: {}\n{}\nVerified runs: {}\nVerified at: {}\nSource events: {}\nReuse when: the same project and branch need this verified workflow.",
        trace.workflow_key,
        trace.command,
        files_line,
        verified_runs,
        verified_at_epoch,
        source_event_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(event_id: i64, verified_at_epoch: i64) -> ProcedureTrace {
        ProcedureTrace {
            project: "/tmp/remem".to_string(),
            branch: Some("main".to_string()),
            workflow_key: "pr-review-loop".to_string(),
            command: "cargo test".to_string(),
            files_touched: vec!["src/lib.rs".to_string()],
            succeeded: true,
            verified_at_epoch,
            source_event_id: Some(event_id),
        }
    }

    #[test]
    fn repeated_verified_workflow_promotes_procedure_memory() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let policy = ProcedurePromotionPolicy::default();
        let candidate =
            build_procedure_candidate(&[trace(10, 1_000), trace(11, 1_100)], 1_200, &policy)
                .expect("two verified traces should promote");

        assert_eq!(candidate.project, "/tmp/remem");
        assert_eq!(candidate.branch.as_deref(), Some("main"));
        assert_eq!(candidate.source_event_ids, vec![10, 11]);
        assert_eq!(candidate.verified_runs, 2);
        assert!(candidate.topic_key.contains("branch-main"));
        assert!(candidate.topic_key.contains("command-cargo-test"));

        let memory_id = promote_procedure_memory(&conn, &candidate)?;
        let (memory_type, branch, evidence): (String, Option<String>, String) = conn.query_row(
            "SELECT memory_type, branch, evidence_event_ids FROM memories WHERE id = ?1",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(memory_type, "procedure");
        assert_eq!(branch.as_deref(), Some("main"));
        assert_eq!(serde_json::from_str::<Vec<i64>>(&evidence)?, vec![10, 11]);
        Ok(())
    }

    #[test]
    fn one_off_verified_workflow_does_not_promote() {
        let policy = ProcedurePromotionPolicy::default();
        let candidate = build_procedure_candidate(&[trace(10, 1_000)], 1_200, &policy);
        assert!(candidate.is_none());
    }

    #[test]
    fn missing_fresh_source_refs_do_not_promote() {
        let policy = ProcedurePromotionPolicy::default();
        let mut missing_source = trace(10, 1_000);
        missing_source.source_event_id = None;
        assert!(
            build_procedure_candidate(&[missing_source, trace(11, 1_050)], 1_100, &policy)
                .is_none()
        );

        let old = trace(12, 1_000);
        let stale_now = 1_000 + DEFAULT_MAX_VERIFICATION_AGE_SECS + 1;
        assert!(
            build_procedure_candidate(&[old, trace(13, stale_now)], stale_now, &policy).is_none()
        );
    }

    #[test]
    fn mixed_project_or_branch_does_not_promote() {
        let policy = ProcedurePromotionPolicy::default();
        let mut other_project = trace(11, 1_100);
        other_project.project = "/tmp/other".to_string();
        assert!(
            build_procedure_candidate(&[trace(10, 1_000), other_project], 1_200, &policy).is_none()
        );

        let mut other_branch = trace(12, 1_100);
        other_branch.branch = Some("feature".to_string());
        assert!(
            build_procedure_candidate(&[trace(10, 1_000), other_branch], 1_200, &policy).is_none()
        );
    }

    #[test]
    fn production_task_promotes_repeated_successful_bash_procedure() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let command = "cargo test";
        for seq in [1, 2] {
            crate::db::record_captured_event(
                &conn,
                &crate::db::CaptureEventInput {
                    host: "codex-cli",
                    session_id: "sess-procedure-runtime",
                    project: "/tmp/remem",
                    cwd: None,
                    event_type: "tool_result",
                    role: None,
                    tool_name: Some("Bash"),
                    content: &serde_json::json!({
                        "seq": seq,
                        "event_type": "bash",
                        "exit_code": 0,
                        "tool_input": { "command": command },
                        "files": "[\"src/lib.rs\"]",
                        "git_branch": "main"
                    })
                    .to_string(),
                    task_kind: Some(crate::db::ExtractionTaskKind::ObservationExtract),
                },
            )?;
        }
        conn.execute("UPDATE workspaces SET git_branch = 'feature'", [])?;
        let task = crate::db::claim_next_extraction_task(&mut conn, "worker-a", 60)?
            .expect("task should be claimed");

        let promoted = promote_verified_procedures_for_task(
            &conn,
            &task,
            &ProcedurePromotionPolicy::default(),
        )?;

        assert_eq!(promoted, 1);
        let (memory_type, topic_key, branch, evidence): (String, String, Option<String>, String) = conn.query_row(
            "SELECT memory_type, topic_key, branch, evidence_event_ids FROM memories WHERE memory_type = 'procedure'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(memory_type, "procedure");
        assert_eq!(branch.as_deref(), Some("main"));
        assert!(topic_key.contains("command-cargo-test"));
        assert_eq!(serde_json::from_str::<Vec<i64>>(&evidence)?.len(), 2);
        Ok(())
    }

    #[test]
    fn production_task_ignores_procedure_events_outside_evidence_window() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let command = "cargo test";
        let mut old_high_watermark = 0;
        for seq in [1, 2] {
            let outcome = crate::db::record_captured_event(
                &conn,
                &crate::db::CaptureEventInput {
                    host: "codex-cli",
                    session_id: "sess-procedure-old",
                    project: "/tmp/remem",
                    cwd: None,
                    event_type: "tool_result",
                    role: None,
                    tool_name: Some("Bash"),
                    content: &serde_json::json!({
                        "seq": seq,
                        "event_type": "bash",
                        "exit_code": 0,
                        "tool_input": { "command": command },
                        "files": "[\"src/lib.rs\"]",
                        "git_branch": "main"
                    })
                    .to_string(),
                    task_kind: None,
                },
            )?;
            old_high_watermark = outcome.event_row_id;
        }
        let current = crate::db::record_captured_event(
            &conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-procedure-current",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: &serde_json::json!({
                    "seq": 3,
                    "event_type": "bash",
                    "exit_code": 0,
                    "tool_input": { "command": command },
                    "files": "[\"src/lib.rs\"]",
                    "git_branch": "main"
                })
                .to_string(),
                task_kind: None,
            },
        )?;
        let (host_id, workspace_id, project_id, session_row_id): (i64, i64, i64, i64) = conn
            .query_row(
                "SELECT host_id, workspace_id, project_id, session_row_id
                 FROM captured_events
                 WHERE id = ?1",
                [current.event_row_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let task = crate::db::ExtractionTask {
            id: 1,
            task_kind: crate::db::ExtractionTaskKind::ObservationExtract,
            host_id,
            workspace_id,
            project_id,
            session_row_id: Some(session_row_id),
            host: "codex-cli".to_string(),
            project: "/tmp/remem".to_string(),
            session_id: Some("sess-procedure-current".to_string()),
            ai_profile: None,
            priority: crate::db::ExtractionTaskKind::ObservationExtract.priority(),
            cursor_event_id: Some(old_high_watermark),
            high_watermark_event_id: Some(current.event_row_id),
            attempts: 0,
            replay_range_id: None,
        };

        let promoted = promote_verified_procedures_for_task(
            &conn,
            &task,
            &ProcedurePromotionPolicy::default(),
        )?;

        assert_eq!(promoted, 0);
        let procedure_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memories WHERE memory_type = 'procedure'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(procedure_count, 0);
        Ok(())
    }

    #[test]
    fn production_task_accumulates_verified_runs_across_windows() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        crate::migrate::run_migrations(&conn)?;
        let command = "cargo test";

        crate::db::record_captured_event(
            &conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-procedure-windowed",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: &serde_json::json!({
                    "event_type": "bash",
                    "exit_code": 0,
                    "tool_input": { "command": command },
                    "files": "[\"src/lib.rs\"]",
                    "git_branch": "main"
                })
                .to_string(),
                task_kind: Some(crate::db::ExtractionTaskKind::ObservationExtract),
            },
        )?;
        let first_task = crate::db::claim_next_extraction_task(&mut conn, "worker-a", 60)?
            .ok_or_else(|| anyhow::anyhow!("first task should be claimed"))?;
        assert_eq!(
            promote_verified_procedures_for_task(
                &conn,
                &first_task,
                &ProcedurePromotionPolicy::default(),
            )?,
            0
        );
        crate::db::mark_extraction_task_done(
            &conn,
            first_task.id,
            "worker-a",
            first_task.high_watermark_event_id,
        )?;

        crate::db::record_captured_event(
            &conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id: "sess-procedure-windowed",
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content: &serde_json::json!({
                    "event_type": "bash",
                    "exit_code": 0,
                    "tool_input": { "command": command },
                    "files": "[\"src/lib.rs\"]",
                    "git_branch": "main"
                })
                .to_string(),
                task_kind: Some(crate::db::ExtractionTaskKind::ObservationExtract),
            },
        )?;
        let second_task = crate::db::claim_next_extraction_task(&mut conn, "worker-b", 60)?
            .ok_or_else(|| anyhow::anyhow!("second task should be claimed"))?;

        assert_eq!(
            promote_verified_procedures_for_task(
                &conn,
                &second_task,
                &ProcedurePromotionPolicy::default(),
            )?,
            1
        );
        let evidence: String = conn.query_row(
            "SELECT evidence_event_ids FROM memories WHERE memory_type = 'procedure'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(serde_json::from_str::<Vec<i64>>(&evidence)?.len(), 2);
        Ok(())
    }
}
