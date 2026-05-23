use anyhow::Result;
use rusqlite::{params, Connection};

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
    let topic_key =
        crate::memory::slugify_for_topic(&format!("procedure {}", first.workflow_key), 96);
    let confidence = (0.7 + (source_event_ids.len() as f64 * 0.08)).min(0.95);
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

pub fn promote_procedure_memory(conn: &Connection, candidate: &ProcedureCandidate) -> Result<i64> {
    let files_json = (!candidate.files.is_empty())
        .then(|| serde_json::to_string(&candidate.files))
        .transpose()?;
    let source_events_json = serde_json::to_string(&candidate.source_event_ids)?;
    let memory_id = crate::memory::insert_memory_full(
        conn,
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
    conn.execute(
        "UPDATE memories
         SET evidence_event_ids = ?1,
             confidence = ?2
         WHERE id = ?3",
        params![source_events_json, candidate.confidence, memory_id],
    )?;
    Ok(memory_id)
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
}
