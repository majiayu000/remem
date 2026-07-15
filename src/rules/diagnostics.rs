use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

use super::{artifact_path_for_project, EvaluationDiagnosticCode};

const EVALUATION_DIAGNOSTIC_VERSION: u32 = 1;
const EVALUATION_MARKER_DIR: &str = ".evaluation-error-sessions";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvaluationErrorMarker {
    version: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    recovered_corruption: bool,
    records: Vec<EvaluationErrorRecord>,
}

impl Default for EvaluationErrorMarker {
    fn default() -> Self {
        Self {
            version: EVALUATION_DIAGNOSTIC_VERSION,
            recovered_corruption: false,
            records: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum EvaluationDiagnosticScope {
    Project,
    Global,
    Unscoped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EvaluationErrorRecord {
    pub occurred_at_epoch: i64,
    scope: EvaluationDiagnosticScope,
    project_key: Option<String>,
    pub codes: Vec<EvaluationDiagnosticCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct EvaluationErrorSnapshot {
    pub latest: Option<EvaluationErrorRecord>,
    pub corrupt_markers: usize,
}

pub(crate) fn evaluation_marker_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("compiled_rules").join(EVALUATION_MARKER_DIR)
}

pub(crate) fn upsert_evaluation_error_record(
    marker_path: &Path,
    data_dir: &Path,
    project: Option<&str>,
    codes: &[EvaluationDiagnosticCode],
) -> Result<bool> {
    let mut codes = codes.to_vec();
    codes.sort_unstable();
    codes.dedup();
    ensure!(!codes.is_empty(), "evaluation error record requires a code");

    let scope = diagnostic_scope(project, &codes);
    let project_key = project.map(|project| project_key(data_dir, project));
    let (mut marker, should_log) = match fs::read(marker_path) {
        Ok(contents) if contents.is_empty() => (EvaluationErrorMarker::default(), false),
        Ok(contents) => match serde_json::from_slice(&contents)
            .context("parse evaluation error session marker")
            .and_then(|marker| {
                validate_marker(&marker)?;
                Ok(marker)
            }) {
            Ok(marker) => (marker, false),
            Err(_) => (
                EvaluationErrorMarker {
                    recovered_corruption: true,
                    ..EvaluationErrorMarker::default()
                },
                false,
            ),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {
            (EvaluationErrorMarker::default(), true)
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "read evaluation error session marker {}",
                    marker_path.display()
                )
            })
        }
    };

    if marker
        .records
        .iter()
        .any(|record| record.scope == scope && record.project_key == project_key)
    {
        return Ok(false);
    }
    marker.records.push(EvaluationErrorRecord {
        occurred_at_epoch: chrono::Utc::now().timestamp(),
        scope,
        project_key,
        codes,
    });
    let mut contents = serde_json::to_vec(&marker)?;
    contents.push(b'\n');
    crate::atomic_file::write_atomic(marker_path, contents)
        .context("publish evaluation error session marker")?;
    crate::log::set_private_permissions(marker_path);
    Ok(should_log)
}

pub(crate) fn load_evaluation_error(
    data_dir: &Path,
    project: &str,
) -> Result<EvaluationErrorSnapshot> {
    let marker_dir = evaluation_marker_dir(data_dir);
    let entries = match fs::read_dir(&marker_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(EvaluationErrorSnapshot::default())
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "read evaluation diagnostic marker directory {}",
                    marker_dir.display()
                )
            })
        }
    };
    let expected_project_key = project_key(data_dir, project);
    let mut snapshot = EvaluationErrorSnapshot::default();
    for entry in entries {
        let entry = entry.context("read evaluation diagnostic marker entry")?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if !entry
            .file_type()
            .context("read evaluation diagnostic marker type")?
            .is_file()
        {
            continue;
        }
        let contents = match fs::read(entry.path()) {
            Ok(contents) => contents,
            Err(_) => {
                snapshot.corrupt_markers += 1;
                continue;
            }
        };
        if contents.is_empty() {
            continue;
        }
        let marker: EvaluationErrorMarker = match serde_json::from_slice(&contents) {
            Ok(marker) => marker,
            Err(_) => {
                snapshot.corrupt_markers += 1;
                continue;
            }
        };
        if validate_marker(&marker).is_err() {
            snapshot.corrupt_markers += 1;
            continue;
        }
        snapshot.corrupt_markers += usize::from(marker.recovered_corruption);
        for record in marker
            .records
            .into_iter()
            .filter(|record| record.matches_project(&expected_project_key))
        {
            if snapshot
                .latest
                .as_ref()
                .is_none_or(|current| record.occurred_at_epoch > current.occurred_at_epoch)
            {
                snapshot.latest = Some(record);
            }
        }
    }
    Ok(snapshot)
}

impl EvaluationErrorRecord {
    fn matches_project(&self, expected_project_key: &str) -> bool {
        match self.scope {
            EvaluationDiagnosticScope::Project => {
                self.project_key.as_deref() == Some(expected_project_key)
            }
            EvaluationDiagnosticScope::Global => true,
            EvaluationDiagnosticScope::Unscoped => false,
        }
    }
}

fn diagnostic_scope(
    project: Option<&str>,
    codes: &[EvaluationDiagnosticCode],
) -> EvaluationDiagnosticScope {
    if project.is_some() {
        EvaluationDiagnosticScope::Project
    } else if codes.contains(&EvaluationDiagnosticCode::Config) {
        EvaluationDiagnosticScope::Global
    } else {
        EvaluationDiagnosticScope::Unscoped
    }
}

fn project_key(data_dir: &Path, project: &str) -> String {
    artifact_path_for_project(data_dir, project)
        .file_stem()
        .expect("compiled rule artifact path always has a file stem")
        .to_string_lossy()
        .into_owned()
}

fn validate_marker(marker: &EvaluationErrorMarker) -> Result<()> {
    ensure!(
        marker.version == EVALUATION_DIAGNOSTIC_VERSION,
        "unsupported evaluation diagnostic version {}",
        marker.version
    );
    for record in &marker.records {
        ensure!(
            record.occurred_at_epoch >= 0,
            "evaluation diagnostic has negative occurred_at_epoch"
        );
        ensure!(
            !record.codes.is_empty(),
            "evaluation diagnostic has no error codes"
        );
        match record.scope {
            EvaluationDiagnosticScope::Project => ensure!(
                record.project_key.is_some(),
                "project evaluation diagnostic has no project key"
            ),
            EvaluationDiagnosticScope::Global | EvaluationDiagnosticScope::Unscoped => ensure!(
                record.project_key.is_none(),
                "non-project evaluation diagnostic has a project key"
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::test_support::test_dir;

    #[test]
    fn session_marker_round_trips_closed_codes_without_project_or_payload() -> Result<()> {
        let data_dir = test_dir("evaluation-error-marker");
        let project = "/private/project";
        let marker = evaluation_marker_dir(&data_dir).join("session-hash");
        assert!(upsert_evaluation_error_record(
            &marker,
            &data_dir,
            Some(project),
            &[
                EvaluationDiagnosticCode::RuleEvaluation,
                EvaluationDiagnosticCode::ArtifactParse,
                EvaluationDiagnosticCode::RuleEvaluation,
            ],
        )?);

        let record = load_evaluation_error(&data_dir, project)?
            .latest
            .context("record")?;
        assert_eq!(
            record.codes,
            vec![
                EvaluationDiagnosticCode::ArtifactParse,
                EvaluationDiagnosticCode::RuleEvaluation
            ]
        );
        let raw = fs::read_to_string(marker)?;
        assert!(!raw.contains(project));
        assert!(!raw.contains("pattern"));
        Ok(())
    }

    #[test]
    fn one_session_records_each_project_but_logs_only_first() -> Result<()> {
        let data_dir = test_dir("evaluation-error-multi-project");
        let marker = evaluation_marker_dir(&data_dir).join("session-hash");
        assert!(upsert_evaluation_error_record(
            &marker,
            &data_dir,
            Some("/repo/one"),
            &[EvaluationDiagnosticCode::ArtifactMissing],
        )?);
        assert!(!upsert_evaluation_error_record(
            &marker,
            &data_dir,
            Some("/repo/two"),
            &[EvaluationDiagnosticCode::ArtifactParse],
        )?);

        assert_eq!(
            load_evaluation_error(&data_dir, "/repo/one")?
                .latest
                .context("project one")?
                .codes,
            vec![EvaluationDiagnosticCode::ArtifactMissing]
        );
        assert_eq!(
            load_evaluation_error(&data_dir, "/repo/two")?
                .latest
                .context("project two")?
                .codes,
            vec![EvaluationDiagnosticCode::ArtifactParse]
        );
        Ok(())
    }

    #[test]
    fn legacy_empty_session_markers_are_ignored() -> Result<()> {
        let data_dir = test_dir("evaluation-error-legacy-marker");
        let marker_dir = evaluation_marker_dir(&data_dir);
        fs::create_dir_all(&marker_dir)?;
        let marker = marker_dir.join("legacy-session-hash");
        fs::File::create(&marker)?;

        assert_eq!(
            load_evaluation_error(&data_dir, "/repo")?,
            EvaluationErrorSnapshot::default()
        );
        assert!(!upsert_evaluation_error_record(
            &marker,
            &data_dir,
            Some("/repo"),
            &[EvaluationDiagnosticCode::ArtifactMissing],
        )?);
        assert_eq!(
            load_evaluation_error(&data_dir, "/repo")?
                .latest
                .context("upgraded legacy marker")?
                .codes,
            vec![EvaluationDiagnosticCode::ArtifactMissing]
        );
        assert!(!fs::read(&marker)?.is_empty());
        Ok(())
    }

    #[test]
    fn corrupt_marker_is_reported_without_hiding_valid_diagnostic() -> Result<()> {
        let data_dir = test_dir("evaluation-error-corrupt-marker");
        let marker_dir = evaluation_marker_dir(&data_dir);
        fs::create_dir_all(&marker_dir)?;
        fs::write(marker_dir.join("corrupt-session"), b"{not-json")?;
        upsert_evaluation_error_record(
            &marker_dir.join("valid-session"),
            &data_dir,
            Some("/repo"),
            &[EvaluationDiagnosticCode::ArtifactParse],
        )?;

        let snapshot = load_evaluation_error(&data_dir, "/repo")?;
        assert_eq!(snapshot.corrupt_markers, 1);
        assert_eq!(
            snapshot.latest.context("valid marker")?.codes,
            vec![EvaluationDiagnosticCode::ArtifactParse]
        );
        Ok(())
    }

    #[test]
    fn corrupt_session_marker_is_recovered_without_relogging() -> Result<()> {
        let data_dir = test_dir("evaluation-error-corrupt-recovery");
        let marker = evaluation_marker_dir(&data_dir).join("session-hash");
        fs::create_dir_all(marker.parent().context("marker directory")?)?;
        fs::write(&marker, b"{private-corrupt-payload")?;

        assert!(!upsert_evaluation_error_record(
            &marker,
            &data_dir,
            Some("/repo"),
            &[EvaluationDiagnosticCode::ArtifactMissing],
        )?);
        let snapshot = load_evaluation_error(&data_dir, "/repo")?;
        assert_eq!(snapshot.corrupt_markers, 1);
        assert_eq!(
            snapshot.latest.context("recovered marker")?.codes,
            vec![EvaluationDiagnosticCode::ArtifactMissing]
        );
        assert!(!upsert_evaluation_error_record(
            &marker,
            &data_dir,
            Some("/repo/two"),
            &[EvaluationDiagnosticCode::ArtifactParse],
        )?);
        assert_eq!(
            load_evaluation_error(&data_dir, "/repo/two")?
                .latest
                .context("second project record")?
                .codes,
            vec![EvaluationDiagnosticCode::ArtifactParse]
        );
        assert!(!fs::read_to_string(marker)?.contains("private-corrupt-payload"));
        Ok(())
    }

    #[test]
    fn global_config_is_visible_but_unscoped_hook_error_is_not() -> Result<()> {
        let data_dir = test_dir("evaluation-error-global-scope");
        let marker_dir = evaluation_marker_dir(&data_dir);
        upsert_evaluation_error_record(
            &marker_dir.join("global-session"),
            &data_dir,
            None,
            &[EvaluationDiagnosticCode::Config],
        )?;
        upsert_evaluation_error_record(
            &marker_dir.join("unscoped-session"),
            &data_dir,
            None,
            &[EvaluationDiagnosticCode::HookInput],
        )?;

        for project in ["/repo/one", "/repo/two"] {
            assert_eq!(
                load_evaluation_error(&data_dir, project)?
                    .latest
                    .context("global marker")?
                    .codes,
                vec![EvaluationDiagnosticCode::Config]
            );
        }
        Ok(())
    }
}
