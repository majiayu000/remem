use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

use super::{artifact_path_for_project, EvaluationDiagnosticCode};

const EVALUATION_DIAGNOSTIC_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationErrorRecord {
    pub version: u32,
    pub occurred_at_epoch: i64,
    pub codes: Vec<EvaluationDiagnosticCode>,
}

pub fn evaluation_error_path(data_dir: &Path, project: &str) -> PathBuf {
    let file_name = artifact_path_for_project(data_dir, project)
        .file_name()
        .expect("compiled rule artifact path always has a file name")
        .to_os_string();
    data_dir
        .join("compiled_rules")
        .join("evaluation_errors")
        .join(file_name)
}

pub fn record_evaluation_error(
    data_dir: &Path,
    project: &str,
    codes: &[EvaluationDiagnosticCode],
) -> Result<()> {
    let mut codes = codes.to_vec();
    codes.sort_unstable();
    codes.dedup();
    ensure!(!codes.is_empty(), "evaluation error record requires a code");

    let record = EvaluationErrorRecord {
        version: EVALUATION_DIAGNOSTIC_VERSION,
        occurred_at_epoch: chrono::Utc::now().timestamp(),
        codes,
    };
    let path = evaluation_error_path(data_dir, project);
    let mut contents = serde_json::to_vec_pretty(&record)?;
    contents.push(b'\n');
    crate::atomic_file::write_atomic(&path, contents)?;
    crate::log::set_private_permissions(&path);
    Ok(())
}

pub fn clear_evaluation_error(data_dir: &Path, project: &str) -> Result<()> {
    let path = evaluation_error_path(data_dir, project);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("remove recovered evaluation diagnostic {}", path.display())),
    }
}

pub fn load_evaluation_error(
    data_dir: &Path,
    project: &str,
) -> Result<Option<EvaluationErrorRecord>> {
    let path = evaluation_error_path(data_dir, project);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read evaluation diagnostic {}", path.display()))
        }
    };
    let record: EvaluationErrorRecord = serde_json::from_str(&contents)
        .with_context(|| format!("parse evaluation diagnostic {}", path.display()))?;
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
    Ok(Some(record))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::test_support::test_dir;

    #[test]
    fn evaluation_error_sidecar_round_trips_closed_codes_only() -> Result<()> {
        let data_dir = test_dir("evaluation-error-sidecar");
        let project = "/private/project";

        record_evaluation_error(
            &data_dir,
            project,
            &[
                EvaluationDiagnosticCode::RuleEvaluation,
                EvaluationDiagnosticCode::ArtifactParse,
                EvaluationDiagnosticCode::RuleEvaluation,
            ],
        )?;

        let record = load_evaluation_error(&data_dir, project)?.context("record")?;
        assert_eq!(
            record.codes,
            vec![
                EvaluationDiagnosticCode::ArtifactParse,
                EvaluationDiagnosticCode::RuleEvaluation
            ]
        );
        let raw = fs::read_to_string(evaluation_error_path(&data_dir, project))?;
        assert!(!raw.contains(project));
        assert!(!raw.contains("pattern"));

        clear_evaluation_error(&data_dir, project)?;
        assert_eq!(load_evaluation_error(&data_dir, project)?, None);
        Ok(())
    }
}
