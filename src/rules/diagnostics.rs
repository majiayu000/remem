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
pub(crate) struct EvaluationErrorRecord {
    pub version: u32,
    pub occurred_at_epoch: i64,
    pub project_key: Option<String>,
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

pub(crate) fn publish_evaluation_error_record(
    marker: &Path,
    data_dir: &Path,
    project: Option<&str>,
    codes: &[EvaluationDiagnosticCode],
) -> Result<()> {
    let mut codes = codes.to_vec();
    codes.sort_unstable();
    codes.dedup();
    ensure!(!codes.is_empty(), "evaluation error record requires a code");

    let record = EvaluationErrorRecord {
        version: EVALUATION_DIAGNOSTIC_VERSION,
        occurred_at_epoch: chrono::Utc::now().timestamp(),
        project_key: project.map(|project| project_key(data_dir, project)),
        codes,
    };
    let mut contents = serde_json::to_vec(&record)?;
    contents.push(b'\n');
    crate::atomic_file::write_atomic(marker, contents)
        .context("publish evaluation error session marker")?;
    crate::log::set_private_permissions(marker);
    Ok(())
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
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with('.') {
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
        let record: EvaluationErrorRecord = match serde_json::from_slice(&contents) {
            Ok(record) => record,
            Err(_) => {
                snapshot.corrupt_markers += 1;
                continue;
            }
        };
        if validate_record(&record).is_err() {
            snapshot.corrupt_markers += 1;
            continue;
        }
        if record
            .project_key
            .as_deref()
            .is_some_and(|key| key != expected_project_key)
        {
            continue;
        }
        if snapshot
            .latest
            .as_ref()
            .is_none_or(|current: &EvaluationErrorRecord| {
                record.occurred_at_epoch > current.occurred_at_epoch
            })
        {
            snapshot.latest = Some(record);
        }
    }
    Ok(snapshot)
}

fn project_key(data_dir: &Path, project: &str) -> String {
    artifact_path_for_project(data_dir, project)
        .file_stem()
        .expect("compiled rule artifact path always has a file stem")
        .to_string_lossy()
        .into_owned()
}

fn validate_record(record: &EvaluationErrorRecord) -> Result<()> {
    ensure!(
        record.version == EVALUATION_DIAGNOSTIC_VERSION,
        "unsupported evaluation diagnostic version {}",
        record.version
    );
    ensure!(
        record.occurred_at_epoch >= 0,
        "evaluation diagnostic has negative occurred_at_epoch"
    );
    ensure!(
        !record.codes.is_empty(),
        "evaluation diagnostic has no error codes"
    );
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
        let marker_dir = evaluation_marker_dir(&data_dir);
        fs::create_dir_all(&marker_dir)?;
        let marker = marker_dir.join("session-hash");
        publish_evaluation_error_record(
            &marker,
            &data_dir,
            Some(project),
            &[
                EvaluationDiagnosticCode::RuleEvaluation,
                EvaluationDiagnosticCode::ArtifactParse,
                EvaluationDiagnosticCode::RuleEvaluation,
            ],
        )?;

        let snapshot = load_evaluation_error(&data_dir, project)?;
        let record = snapshot.latest.context("record")?;
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
    fn legacy_empty_session_markers_are_ignored() -> Result<()> {
        let data_dir = test_dir("evaluation-error-legacy-marker");
        let marker_dir = evaluation_marker_dir(&data_dir);
        fs::create_dir_all(&marker_dir)?;
        fs::File::create(marker_dir.join("legacy-session-hash"))?;

        assert_eq!(
            load_evaluation_error(&data_dir, "/repo")?,
            EvaluationErrorSnapshot::default()
        );
        Ok(())
    }

    #[test]
    fn corrupt_marker_is_reported_without_hiding_valid_diagnostic() -> Result<()> {
        let data_dir = test_dir("evaluation-error-corrupt-marker");
        let marker_dir = evaluation_marker_dir(&data_dir);
        fs::create_dir_all(&marker_dir)?;
        fs::write(marker_dir.join("corrupt-session"), b"{not-json")?;
        publish_evaluation_error_record(
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
    fn global_config_error_is_visible_from_every_project() -> Result<()> {
        let data_dir = test_dir("evaluation-error-global-config");
        let marker_dir = evaluation_marker_dir(&data_dir);
        fs::create_dir_all(&marker_dir)?;
        publish_evaluation_error_record(
            &marker_dir.join("global-session"),
            &data_dir,
            None,
            &[EvaluationDiagnosticCode::Config],
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
