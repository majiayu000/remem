use anyhow::{Context, Result};
use rusqlite::{types::Value, Connection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifiedProcedureEvidence {
    pub(super) source_event_ids: Vec<i64>,
    pub(super) verified_runs: usize,
    pub(super) last_verification_epoch: i64,
    pub(super) branch: Option<String>,
    pub(super) workflow_key: String,
    pub(super) command: String,
    pub(super) files_touched: Vec<String>,
}

impl VerifiedProcedureEvidence {
    pub(super) fn title(&self) -> String {
        format!("Procedure: {}", self.workflow_key)
    }

    pub(super) fn canonical_content(&self) -> String {
        let files_line = if self.files_touched.is_empty() {
            "Files: none recorded".to_string()
        } else {
            format!("Files: {}", self.files_touched.join(", "))
        };
        format!(
            "Procedure: {}\nCommand: {}\n{}\nVerified runs: {}\nVerified at: {}\nSource events: {}\nReuse when: the same project and branch need this verified workflow.",
            self.workflow_key,
            self.command,
            files_line,
            self.verified_runs,
            self.last_verification_epoch,
            self.source_event_ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub(super) fn reuse_condition(&self) -> String {
        match self.branch.as_deref() {
            Some(branch) => format!(
                "the same project and branch '{branch}' need verified workflow '{}'.",
                self.workflow_key
            ),
            None => format!(
                "the same project needs verified workflow '{}'.",
                self.workflow_key
            ),
        }
    }

    pub(super) fn confidence(&self) -> f64 {
        super::confidence_for_verified_runs(self.verified_runs)
    }
}

pub(super) fn parse_evidence_ids(raw: Option<&str>) -> Result<Vec<i64>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).with_context(|| "invalid procedure evidence_event_ids JSON")
}

pub(super) fn load_verified_procedure_evidence(
    conn: &Connection,
    evidence_ids: &[i64],
    memory_project: &str,
    policy: &super::ProcedurePromotionPolicy,
) -> Result<Option<VerifiedProcedureEvidence>> {
    if evidence_ids.is_empty() {
        return Ok(None);
    }
    let earliest = chrono::Utc::now()
        .timestamp()
        .saturating_sub(policy.max_verification_age_secs);
    let placeholders = (1..=evidence_ids.len())
        .map(|idx| format!("?{idx}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT p.project_path, v.branch, v.workflow_key, v.command, v.files_touched,
                v.source_event_id, v.verified_at_epoch
         FROM procedure_verifications v
         JOIN projects p ON p.id = v.project_id
         WHERE v.source_event_id IN ({placeholders})
           AND v.verified_at_epoch >= ?
         ORDER BY v.verified_at_epoch ASC, v.source_event_id ASC"
    );
    let mut params = evidence_ids
        .iter()
        .map(|id| Value::Integer(*id))
        .collect::<Vec<_>>();
    params.push(Value::Integer(earliest));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |row| {
            Ok(VerificationRow {
                project: row.get(0)?,
                branch: row.get(1)?,
                workflow_key: row.get(2)?,
                command: row.get(3)?,
                files_touched: row.get(4)?,
                source_event_id: row.get(5)?,
                verified_at_epoch: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    if first.project != memory_project {
        return Ok(None);
    }
    if rows.iter().any(|row| {
        row.project != first.project
            || row.branch != first.branch
            || row.workflow_key != first.workflow_key
            || row.command != first.command
    }) {
        return Ok(None);
    }

    let branch = first.branch.clone();
    let workflow_key = first.workflow_key.clone();
    let command = first.command.clone();
    let mut source_ids = std::collections::BTreeSet::new();
    let mut files_touched = std::collections::BTreeSet::new();
    let mut last_verification_epoch = first.verified_at_epoch;
    for row in rows {
        source_ids.insert(row.source_event_id);
        last_verification_epoch = last_verification_epoch.max(row.verified_at_epoch);
        for file in parse_files(Some(&row.files_touched))? {
            files_touched.insert(file);
        }
    }
    let verified_runs = source_ids.len();
    let source_event_ids = source_ids.into_iter().collect();
    Ok(Some(VerifiedProcedureEvidence {
        source_event_ids,
        verified_runs,
        last_verification_epoch,
        branch,
        workflow_key,
        command,
        files_touched: files_touched.into_iter().collect(),
    }))
}

struct VerificationRow {
    project: String,
    branch: Option<String>,
    workflow_key: String,
    command: String,
    files_touched: String,
    source_event_id: i64,
    verified_at_epoch: i64,
}

fn parse_files(raw: Option<&str>) -> Result<Vec<String>> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).with_context(|| "invalid procedure files JSON")
}
