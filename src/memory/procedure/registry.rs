use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use super::{load_export_eligible_procedure, ProcedureExportFormat, ProcedureExportSource};

const SOURCE_DIGEST_VERSION: i64 = 1;

pub(crate) struct ProcedureExportRecordRequest<'a> {
    pub(crate) source: &'a ProcedureExportSource,
    pub(crate) format: ProcedureExportFormat,
    pub(crate) output_path: &'a Path,
    pub(crate) content: &'a str,
    pub(crate) cwd: &'a Path,
    pub(crate) exported_at_epoch: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProcedureExportDoctorReport {
    pub(crate) total_exports: usize,
    pub(crate) project_count: usize,
    pub(crate) project_exports: Vec<ProcedureExportProjectCount>,
    pub(crate) inactive: usize,
    pub(crate) stale: usize,
    pub(crate) changed: usize,
    pub(crate) examples: Vec<ProcedureExportDriftExample>,
}

impl ProcedureExportDoctorReport {
    pub(crate) fn drifted_exports(&self) -> usize {
        self.inactive + self.stale + self.changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcedureExportProjectCount {
    pub(crate) project: String,
    pub(crate) exports: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProcedureExportDriftExample {
    pub(crate) memory_id: i64,
    pub(crate) format: String,
    pub(crate) output_path: String,
    pub(crate) reason: ProcedureExportDriftReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcedureExportDriftReason {
    SourceInactive,
    VerificationStale,
    SourceChanged,
}

impl ProcedureExportDriftReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SourceInactive => "source procedure inactive",
            Self::VerificationStale => "source verification stale",
            Self::SourceChanged => "source procedure changed after export",
        }
    }
}

pub(crate) fn record_procedure_export(
    conn: &Connection,
    request: ProcedureExportRecordRequest<'_>,
) -> Result<()> {
    let now = request.exported_at_epoch;
    let output_path = registry_output_path(request.output_path, request.cwd);
    let content_digest = crate::db::content_identity_hash(request.content.as_bytes());
    let source_digest = source_digest_for_export_source(request.source)?;
    conn.execute(
        "INSERT INTO procedure_exports
         (memory_id, project, format, output_path, content_digest, source_digest,
          source_digest_version, source_updated_at_epoch, exported_at_epoch,
          remem_version, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?9, ?9)
         ON CONFLICT(memory_id, format, output_path) DO UPDATE SET
             project = excluded.project,
             content_digest = excluded.content_digest,
             source_digest = excluded.source_digest,
             source_digest_version = excluded.source_digest_version,
             source_updated_at_epoch = excluded.source_updated_at_epoch,
             exported_at_epoch = excluded.exported_at_epoch,
             remem_version = excluded.remem_version,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            request.source.id,
            request.source.project,
            request.format.as_str(),
            output_path,
            content_digest,
            source_digest,
            SOURCE_DIGEST_VERSION,
            request.source.source_updated_at_epoch,
            now,
            crate::build_info::package_version(),
        ],
    )
    .context("record procedure export registry row")?;
    Ok(())
}

pub(crate) fn ensure_existing_export_registry_match(
    conn: &Connection,
    source: &ProcedureExportSource,
    format: ProcedureExportFormat,
    output_path: &Path,
    cwd: &Path,
    existing_content: &str,
) -> Result<()> {
    let output_path = registry_output_path(output_path, cwd);
    let content_digest = crate::db::content_identity_hash(existing_content.as_bytes());
    let recorded_digest = conn
        .query_row(
            "SELECT content_digest
             FROM procedure_exports
             WHERE memory_id = ?1
               AND format = ?2
               AND output_path = ?3",
            params![source.id, format.as_str(), output_path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("load existing procedure export registry row")?;

    let Some(recorded_digest) = recorded_digest else {
        bail!(
            "procedure export target already exists without a matching registry row for memory #{} {}; choose --out <new-dir> or rename the existing draft",
            source.id,
            output_path
        );
    };
    if recorded_digest != content_digest {
        bail!(
            "procedure export target digest no longer matches registry row for memory #{} {}; choose --out <new-dir> or rename the existing draft",
            source.id,
            output_path
        );
    }
    Ok(())
}

pub(crate) fn procedure_export_registry_exists(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'procedure_exports'",
        [],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
    .context("check procedure export registry availability")
}

pub(crate) fn load_procedure_export_doctor_report(
    conn: &Connection,
    now_epoch: i64,
) -> Result<ProcedureExportDoctorReport> {
    let rows = load_registry_rows(conn)?;
    let mut projects = BTreeMap::<String, usize>::new();
    let mut report = ProcedureExportDoctorReport {
        total_exports: rows.len(),
        ..ProcedureExportDoctorReport::default()
    };

    for row in rows {
        *projects.entry(row.project.clone()).or_default() += 1;
        if let Some(reason) = classify_drift(conn, &row, now_epoch)? {
            match reason {
                ProcedureExportDriftReason::SourceInactive => report.inactive += 1,
                ProcedureExportDriftReason::VerificationStale => report.stale += 1,
                ProcedureExportDriftReason::SourceChanged => report.changed += 1,
            }
            if report.examples.len() < 3 {
                report.examples.push(ProcedureExportDriftExample {
                    memory_id: row.memory_id,
                    format: row.format,
                    output_path: row.output_path,
                    reason,
                });
            }
        }
    }

    report.project_count = projects.len();
    report.project_exports = projects
        .into_iter()
        .map(|(project, exports)| ProcedureExportProjectCount { project, exports })
        .collect();
    Ok(report)
}

fn classify_drift(
    conn: &Connection,
    row: &ProcedureExportRegistryRow,
    now_epoch: i64,
) -> Result<Option<ProcedureExportDriftReason>> {
    let Some(source_state) = load_source_state(conn, row.memory_id)? else {
        return Ok(Some(ProcedureExportDriftReason::SourceInactive));
    };
    if source_state.memory_type != "procedure"
        || source_state.status != "active"
        || source_state
            .expires_at_epoch
            .is_some_and(|expires_at| expires_at <= now_epoch)
    {
        return Ok(Some(ProcedureExportDriftReason::SourceInactive));
    }

    match load_export_eligible_procedure(conn, row.memory_id) {
        Ok(source) => {
            let current_source_digest = source_digest_for_export_source(&source)?;
            if row.source_digest_version != SOURCE_DIGEST_VERSION
                || row.source_updated_at_epoch != source.source_updated_at_epoch
                || row.source_digest != current_source_digest
            {
                return Ok(Some(ProcedureExportDriftReason::SourceChanged));
            }
            Ok(None)
        }
        Err(error) => {
            let message = error.to_string();
            if message.contains("fresh verification evidence")
                || message.contains("fresh verified run")
            {
                Ok(Some(ProcedureExportDriftReason::VerificationStale))
            } else if message.contains("policy-suppressed")
                || message.contains("not current")
                || message.contains("superseded")
                || message.contains("source status")
                || message.contains("source is expired")
            {
                Ok(Some(ProcedureExportDriftReason::SourceInactive))
            } else {
                Ok(Some(ProcedureExportDriftReason::SourceChanged))
            }
        }
    }
}

#[derive(Debug)]
struct ProcedureExportRegistryRow {
    memory_id: i64,
    project: String,
    format: String,
    output_path: String,
    source_digest: String,
    source_digest_version: i64,
    source_updated_at_epoch: i64,
}

fn load_registry_rows(conn: &Connection) -> Result<Vec<ProcedureExportRegistryRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, project, format, output_path, source_digest,
                    source_digest_version, source_updated_at_epoch
             FROM procedure_exports
             ORDER BY exported_at_epoch DESC, id DESC",
        )
        .context("load procedure export registry rows")?;
    let rows = stmt.query_map([], |row| {
        Ok(ProcedureExportRegistryRow {
            memory_id: row.get(0)?,
            project: row.get(1)?,
            format: row.get(2)?,
            output_path: row.get(3)?,
            source_digest: row.get(4)?,
            source_digest_version: row.get(5)?,
            source_updated_at_epoch: row.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>()
        .context("read procedure export registry rows")
}

#[derive(Debug)]
struct ProcedureSourceState {
    memory_type: String,
    status: String,
    expires_at_epoch: Option<i64>,
}

fn load_source_state(conn: &Connection, memory_id: i64) -> Result<Option<ProcedureSourceState>> {
    conn.query_row(
        "SELECT memory_type, status, expires_at_epoch
         FROM memories
         WHERE id = ?1",
        params![memory_id],
        |row| {
            Ok(ProcedureSourceState {
                memory_type: row.get(0)?,
                status: row.get(1)?,
                expires_at_epoch: row.get(2)?,
            })
        },
    )
    .optional()
    .context("load procedure export source state")
}

#[derive(Serialize)]
struct ProcedureSourceDigestV1<'a> {
    project: &'a str,
    branch: &'a Option<String>,
    topic_key: &'a Option<String>,
    title: &'a str,
    canonical_content: &'a str,
    workflow_key: &'a str,
    command: &'a str,
    reuse_condition: &'a str,
    files_touched: &'a [String],
    evidence_event_ids: &'a [i64],
    verified_runs: usize,
    last_verification_epoch: i64,
    confidence_basis_points: i64,
}

fn source_digest_for_export_source(source: &ProcedureExportSource) -> Result<String> {
    let snapshot = ProcedureSourceDigestV1 {
        project: &source.project,
        branch: &source.branch,
        topic_key: &source.topic_key,
        title: &source.title,
        canonical_content: &source.canonical_content,
        workflow_key: &source.workflow_key,
        command: &source.command,
        reuse_condition: &source.reuse_condition,
        files_touched: &source.files_touched,
        evidence_event_ids: &source.evidence_event_ids,
        verified_runs: source.verified_runs,
        last_verification_epoch: source.last_verification_epoch,
        confidence_basis_points: (source.confidence * 10_000.0).round() as i64,
    };
    let bytes =
        serde_json::to_vec(&snapshot).context("serialize procedure export source digest")?;
    Ok(crate::db::content_identity_hash(&bytes))
}

fn registry_output_path(path: &Path, cwd: &Path) -> String {
    let cwd = normalize_path_lexically(cwd);
    let absolute = if path.is_absolute() {
        normalize_path_lexically(path)
    } else {
        normalize_path_lexically(&cwd.join(path))
    };
    let relative = absolute
        .strip_prefix(&cwd)
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| relative_path_lexically(&absolute, &cwd).unwrap_or(absolute));
    normalize_path_separators(&relative)
}

fn normalize_path_separators(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn relative_path_lexically(path: &Path, base: &Path) -> Option<PathBuf> {
    let path_components = comparable_components(path)?;
    let base_components = comparable_components(base)?;
    let mut common = 0;
    while common < path_components.len()
        && common < base_components.len()
        && path_components[common] == base_components[common]
    {
        common += 1;
    }
    if common == 0 {
        return None;
    }

    let mut relative = PathBuf::new();
    for _ in base_components[common..].iter().filter(|component| {
        component.as_os_str() != std::ffi::OsStr::new("/")
            && !component.as_os_str().to_string_lossy().ends_with(':')
    }) {
        relative.push("..");
    }
    for component in &path_components[common..] {
        relative.push(component);
    }
    if relative.as_os_str().is_empty() {
        relative.push(".");
    }
    Some(relative)
}

fn comparable_components(path: &Path) -> Option<Vec<OsString>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => components.push(prefix.as_os_str().to_os_string()),
            Component::RootDir => components.push(std::ffi::OsStr::new("/").to_os_string()),
            Component::Normal(value) => components.push(value.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => return None,
        }
    }
    Some(components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn migrations_create_procedure_exports_registry() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;

        let table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'procedure_exports'",
            [],
            |row| row.get(0),
        )?;
        let source_digest_version_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('procedure_exports')
             WHERE name = 'source_digest_version'",
            [],
            |row| row.get(0),
        )?;

        assert_eq!(table_count, 1);
        assert_eq!(source_digest_version_count, 1);
        assert_eq!(crate::migrate::latest_schema_version(), 63);
        Ok(())
    }

    #[test]
    fn record_procedure_export_upserts_source_and_content_snapshots() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status, scope)
             VALUES (42, '/tmp/remem', 'Procedure: cargo-test', 'Procedure: cargo-test',
                     'procedure', 1, 1, 'active', 'project')",
            [],
        )?;
        let source = registry_fixture_source();
        let cwd = Path::new("/repo");
        let output_path = Path::new("/repo/remem-drafts/cargo-test.runbook.md");

        record_procedure_export(
            &conn,
            ProcedureExportRecordRequest {
                source: &source,
                format: ProcedureExportFormat::RunbookMd,
                output_path,
                content: "first",
                cwd,
                exported_at_epoch: 10,
            },
        )?;
        record_procedure_export(
            &conn,
            ProcedureExportRecordRequest {
                source: &source,
                format: ProcedureExportFormat::RunbookMd,
                output_path,
                content: "second",
                cwd,
                exported_at_epoch: 20,
            },
        )?;

        let (count, output, content_digest, exported_at): (i64, String, String, i64) = conn
            .query_row(
                "SELECT COUNT(*), output_path, content_digest, exported_at_epoch
                 FROM procedure_exports",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;

        assert_eq!(count, 1);
        assert_eq!(output, "remem-drafts/cargo-test.runbook.md");
        assert_eq!(content_digest, crate::db::content_identity_hash(b"second"));
        assert_eq!(exported_at, 20);
        Ok(())
    }

    #[test]
    fn ensure_existing_export_registry_match_rejects_missing_or_mismatched_rows() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status, scope)
             VALUES (42, '/tmp/remem', 'Procedure: cargo-test', 'Procedure: cargo-test',
                     'procedure', 1, 1, 'active', 'project')",
            [],
        )?;
        let source = registry_fixture_source();
        let cwd = Path::new("/repo");
        let output_path = Path::new("/repo/remem-drafts/cargo-test.runbook.md");

        let missing = ensure_existing_export_registry_match(
            &conn,
            &source,
            ProcedureExportFormat::RunbookMd,
            output_path,
            cwd,
            "first",
        )
        .expect_err("missing registry row must block overwrite");
        assert!(missing
            .to_string()
            .contains("without a matching registry row"));

        record_procedure_export(
            &conn,
            ProcedureExportRecordRequest {
                source: &source,
                format: ProcedureExportFormat::RunbookMd,
                output_path,
                content: "first",
                cwd,
                exported_at_epoch: 10,
            },
        )?;

        ensure_existing_export_registry_match(
            &conn,
            &source,
            ProcedureExportFormat::RunbookMd,
            output_path,
            cwd,
            "first",
        )?;
        let mismatch = ensure_existing_export_registry_match(
            &conn,
            &source,
            ProcedureExportFormat::RunbookMd,
            output_path,
            cwd,
            "edited",
        )
        .expect_err("digest mismatch must block overwrite");
        assert!(mismatch.to_string().contains("digest no longer matches"));
        Ok(())
    }

    #[test]
    fn registry_output_path_is_relative_outside_cwd() {
        assert_eq!(
            registry_output_path(
                Path::new("/repo/../other/procedure.runbook.md"),
                Path::new("/repo/project")
            ),
            "../../other/procedure.runbook.md"
        );
        assert_eq!(
            registry_output_path(
                Path::new("remem-drafts/procedure.runbook.md"),
                Path::new("/repo")
            ),
            "remem-drafts/procedure.runbook.md"
        );
    }

    fn registry_fixture_source() -> ProcedureExportSource {
        ProcedureExportSource {
            id: 42,
            project: "/tmp/remem".to_string(),
            branch: Some("main".to_string()),
            topic_key: Some("procedure-cargo-test".to_string()),
            title: "Procedure: cargo-test".to_string(),
            stored_title: "Procedure: cargo-test".to_string(),
            canonical_content: "Procedure: cargo-test\nCommand: cargo test\nFiles: src/lib.rs\nVerified runs: 2\nVerified at: 1700000000\nSource events: 1,2\nReuse when: the same project and branch need this verified workflow.".to_string(),
            workflow_key: "cargo-test".to_string(),
            command: "cargo test".to_string(),
            reuse_condition: "the same project and branch need this verified workflow.".to_string(),
            files_touched: vec!["src/lib.rs".to_string()],
            evidence_event_ids: vec![1, 2],
            verified_runs: 2,
            last_verification_epoch: 1_700_000_000,
            confidence: 0.86,
            source_updated_at_epoch: 1_700_000_100,
        }
    }
}
