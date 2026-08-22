use anyhow::{bail, Result};
use rusqlite::Connection;
use serde::Serialize;

use super::{payload_sha256, ActivationPoisoningVerdict, ActiveMemoryWriteRequest};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExpectedActiveMemory {
    pub title: String,
    pub content: String,
    pub memory_type: String,
    pub topic_key: Option<String>,
    pub files: Option<String>,
    pub evidence_event_ids: Option<String>,
    pub source_candidate_id: Option<i64>,
}

impl ExpectedActiveMemory {
    pub(crate) fn new(title: &str, content: &str, memory_type: &str) -> Self {
        Self {
            title: title.to_string(),
            content: content.to_string(),
            memory_type: memory_type.to_string(),
            topic_key: None,
            files: None,
            evidence_event_ids: None,
            source_candidate_id: None,
        }
    }

    pub(crate) fn with_topic_key(mut self, topic_key: Option<&str>) -> Self {
        self.topic_key = topic_key.map(str::to_string);
        self
    }

    pub(crate) fn with_files(mut self, files: Option<&str>) -> Self {
        self.files = files.map(str::to_string);
        self
    }

    pub(crate) fn with_candidate_evidence(
        mut self,
        evidence_event_ids: Option<&str>,
        source_candidate_id: Option<i64>,
    ) -> Self {
        self.evidence_event_ids = evidence_event_ids.map(str::to_string);
        self.source_candidate_id = source_candidate_id;
        self
    }

    pub(crate) fn from_existing(conn: &Connection, memory_id: i64) -> Result<Self> {
        conn.query_row(
            "SELECT title, content, memory_type, topic_key, files,
                    evidence_event_ids, source_candidate_id
             FROM memories WHERE id = ?1",
            [memory_id],
            |row| {
                Ok(Self {
                    title: row.get(0)?,
                    content: row.get(1)?,
                    memory_type: row.get(2)?,
                    topic_key: row.get(3)?,
                    files: row.get(4)?,
                    evidence_event_ids: row.get(5)?,
                    source_candidate_id: row.get(6)?,
                })
            },
        )
        .map_err(Into::into)
    }

    pub(crate) fn with_content(mut self, content: &str) -> Self {
        self.content = content.to_string();
        self
    }

    pub(crate) fn sha256(&self) -> String {
        payload_sha256(&[
            &self.title,
            &self.content,
            &self.memory_type,
            if self.topic_key.is_some() { "1" } else { "0" },
            self.topic_key.as_deref().unwrap_or(""),
            if self.files.is_some() { "1" } else { "0" },
            self.files.as_deref().unwrap_or(""),
            if self.evidence_event_ids.is_some() {
                "1"
            } else {
                "0"
            },
            self.evidence_event_ids.as_deref().unwrap_or(""),
            if self.source_candidate_id.is_some() {
                "1"
            } else {
                "0"
            },
            &self.source_candidate_id.unwrap_or_default().to_string(),
        ])
    }
}

pub(super) fn validate_result_payload(
    conn: &Connection,
    memory_id: i64,
    request: &ActiveMemoryWriteRequest,
) -> Result<String> {
    let actual = ExpectedActiveMemory::from_existing(conn, memory_id)?;
    if actual != request.expected_memory {
        let mut fields = Vec::new();
        if actual.title != request.expected_memory.title {
            fields.push("title");
        }
        if actual.content != request.expected_memory.content {
            fields.push("content");
        }
        if actual.memory_type != request.expected_memory.memory_type {
            fields.push("memory_type");
        }
        if actual.topic_key != request.expected_memory.topic_key {
            fields.push("topic_key");
        }
        if actual.files != request.expected_memory.files {
            fields.push("files");
        }
        if actual.evidence_event_ids != request.expected_memory.evidence_event_ids {
            fields.push("evidence_event_ids");
        }
        if actual.source_candidate_id != request.expected_memory.source_candidate_id {
            fields.push("source_candidate_id");
        }
        bail!(
            "memory activation result payload does not match reviewed request: {}",
            fields.join(",")
        );
    }

    validate_poisoning_verdict(conn, memory_id, &actual, request.poisoning_verdict)?;
    Ok(actual.sha256())
}

pub(super) fn validate_poisoning_verdict(
    conn: &Connection,
    memory_id: i64,
    actual: &ExpectedActiveMemory,
    verdict: ActivationPoisoningVerdict,
) -> Result<()> {
    let (acknowledged_pattern_id, acknowledged_pattern_version, acknowledged_at_epoch): (
        Option<String>,
        Option<i64>,
        Option<i64>,
    ) = conn.query_row(
        "SELECT acknowledged_pattern_id, acknowledged_pattern_version, acknowledged_at_epoch
             FROM memories WHERE id = ?1",
        [memory_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let matched = crate::memory::poisoning::scan_instruction_pattern(&format!(
        "{}\n{}",
        actual.title, actual.content
    ));
    match verdict {
        ActivationPoisoningVerdict::Clean | ActivationPoisoningVerdict::UpstreamValidated => {
            if let Some(matched) = matched {
                bail!(
                    "memory activation result matched instruction-pattern {}@{} after validation",
                    matched.pattern_id,
                    matched.pattern_set_version
                );
            }
        }
        ActivationPoisoningVerdict::Acknowledged => {
            if acknowledged_pattern_id.as_deref().is_none_or(str::is_empty)
                || acknowledged_pattern_version.is_none_or(|version| version <= 0)
                || acknowledged_at_epoch.is_none_or(|epoch| epoch <= 0)
            {
                bail!("acknowledged activation did not persist acknowledgement evidence");
            }
            if matched.is_some_and(|matched| {
                acknowledged_pattern_id.as_deref() != Some(matched.pattern_id)
                    || acknowledged_pattern_version != Some(matched.pattern_set_version)
            }) {
                bail!("memory activation acknowledgement does not match stored payload");
            }
        }
        ActivationPoisoningVerdict::ExactRecovery => {
            let acknowledgement_absent = acknowledged_pattern_id.is_none()
                && acknowledged_pattern_version.is_none()
                && acknowledged_at_epoch.is_none();
            let acknowledgement_complete = acknowledged_pattern_id
                .as_deref()
                .is_some_and(|pattern_id| !pattern_id.is_empty())
                && acknowledged_pattern_version.is_some_and(|version| version > 0)
                && acknowledged_at_epoch.is_some_and(|epoch| epoch > 0);
            if !acknowledgement_absent && !acknowledgement_complete {
                bail!("exact recovery restored incomplete acknowledgement evidence");
            }
            if let Some(matched) = matched {
                if acknowledged_pattern_id.as_deref() != Some(matched.pattern_id)
                    || acknowledged_pattern_version != Some(matched.pattern_set_version)
                {
                    bail!("exact recovery restored unacknowledged instruction-pattern payload");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::{bail, Result};
    use rusqlite::Connection;

    use super::*;
    use crate::memory::activation::{
        execute_one, ActivationActorKind, ActivationProvenanceKind, ActivationRouteKind,
        ActiveMemoryRoute, ActiveMemoryWriteRequest,
    };
    use crate::memory::poisoning::SourceTrustClass;

    fn request(id: &str, content: &str) -> ActiveMemoryWriteRequest {
        ActiveMemoryWriteRequest {
            activation_id: id.to_string(),
            route_kind: ActivationRouteKind::RustApi,
            actor_kind: ActivationActorKind::RustApi,
            source_operation: "save_memory".to_string(),
            source_trust: SourceTrustClass::LocalToolOutput,
            result_source_trust: SourceTrustClass::LocalToolOutput,
            source_project: "/repo".to_string(),
            route: ActiveMemoryRoute::default_for("/repo", None, "project"),
            provenance_kind: ActivationProvenanceKind::RustApi,
            provenance_ref: "rust-api:test".to_string(),
            payload_sha256: payload_sha256(&[content]),
            expected_memory: ExpectedActiveMemory::new("title", content, "discovery"),
            poisoning_verdict: ActivationPoisoningVerdict::Clean,
            superseded_ids: Vec::new(),
        }
    }

    fn insert_memory(conn: &Connection, content: &str) -> Result<i64> {
        conn.execute(
            "INSERT INTO memories
             (project, title, content, memory_type, created_at_epoch,
              updated_at_epoch, status, scope, source_project, target_project,
              owner_scope, owner_key, context_class, source_trust_class)
             VALUES ('/repo', 'title', ?1, 'discovery', 1, 1, 'active',
                     'project', '/repo', '/repo', 'repo', '/repo',
                     'startup_core', 'local_tool_output')",
            [content],
        )?;
        Ok(conn.last_insert_rowid())
    }

    #[test]
    fn writer_cannot_persist_a_payload_different_from_the_reviewed_request() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let error = execute_one(&conn, &request("save:bound", "reviewed"), |_| {
            insert_memory(&conn, "different")
        })
        .expect_err("different result payload must roll back");
        assert!(error.to_string().contains("reviewed request"));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row
                .get::<_, i64>(0))?,
            0
        );
        Ok(())
    }

    #[test]
    fn replay_rejects_a_result_that_is_no_longer_active() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        let request = request("save:inactive", "same");
        let first = execute_one(&conn, &request, |_| insert_memory(&conn, "same"))?;
        conn.execute(
            "UPDATE memories SET status = 'archived' WHERE id = ?1",
            [first.memory_id],
        )?;
        let error = execute_one(&conn, &request, |_| bail!("writer must not replay"))
            .expect_err("inactive result must not replay as a save success");
        assert!(error
            .to_string()
            .contains("inactive without a superseding receipt"));
        Ok(())
    }
}
