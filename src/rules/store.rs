use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::rules::artifact::CompiledRulesArtifact;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactLoad {
    Loaded(CompiledRulesArtifact),
    FailOpen {
        kind: ArtifactLoadErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactLoadErrorKind {
    Missing,
    Read,
    Parse,
    Validate,
}

pub fn artifact_path_for_project(data_dir: impl AsRef<Path>, project: &str) -> PathBuf {
    data_dir
        .as_ref()
        .join("compiled_rules")
        .join(format!("{}.json", project_hash(project)))
}

pub fn write_artifact_atomic(
    path: impl AsRef<Path>,
    artifact: &CompiledRulesArtifact,
) -> Result<()> {
    artifact.validate()?;
    let mut contents = serde_json::to_vec_pretty(artifact)?;
    contents.push(b'\n');
    crate::atomic_file::write_atomic(path, contents)
}

pub fn load_artifact_fail_open(path: impl AsRef<Path>) -> ArtifactLoad {
    let path = path.as_ref();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return ArtifactLoad::FailOpen {
                kind: ArtifactLoadErrorKind::Missing,
                message: format!("compiled rules artifact missing: {}", path.display()),
            };
        }
        Err(err) => {
            return ArtifactLoad::FailOpen {
                kind: ArtifactLoadErrorKind::Read,
                message: format!(
                    "read compiled rules artifact {} failed: {err}",
                    path.display()
                ),
            };
        }
    };

    let artifact = match serde_json::from_str::<CompiledRulesArtifact>(&text) {
        Ok(artifact) => artifact,
        Err(err) => {
            return ArtifactLoad::FailOpen {
                kind: ArtifactLoadErrorKind::Parse,
                message: format!(
                    "parse compiled rules artifact {} failed: {err}",
                    path.display()
                ),
            };
        }
    };

    match artifact.validate() {
        Ok(()) => ArtifactLoad::Loaded(artifact),
        Err(err) => ArtifactLoad::FailOpen {
            kind: ArtifactLoadErrorKind::Validate,
            message: format!(
                "validate compiled rules artifact {} failed: {err}",
                path.display()
            ),
        },
    }
}

fn project_hash(project: &str) -> String {
    let digest = Sha256::digest(project.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::artifact::ARTIFACT_VERSION;
    use crate::rules::test_support::{package_manager_artifact, test_dir};

    #[test]
    fn artifact_path_uses_stable_project_hash() {
        let left = artifact_path_for_project("/tmp/remem", "/workspace/project");
        let right = artifact_path_for_project("/tmp/remem", "/workspace/project");
        let other = artifact_path_for_project("/tmp/remem", "/workspace/other");

        assert_eq!(left, right);
        assert_ne!(left, other);
        assert_eq!(
            left.parent().and_then(Path::file_name),
            Some("compiled_rules".as_ref())
        );
        assert_eq!(left.extension().and_then(|ext| ext.to_str()), Some("json"));
    }

    #[test]
    fn write_and_load_artifact_round_trip() -> Result<()> {
        let dir = test_dir("round-trip");
        let path = dir.join("artifact.json");
        let artifact = package_manager_artifact();

        write_artifact_atomic(&path, &artifact)?;
        let loaded = load_artifact_fail_open(&path);

        assert_eq!(loaded, ArtifactLoad::Loaded(artifact));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn load_missing_artifact_fails_open() {
        let path = test_dir("missing").join("artifact.json");

        let loaded = load_artifact_fail_open(&path);

        assert!(matches!(
            loaded,
            ArtifactLoad::FailOpen {
                kind: ArtifactLoadErrorKind::Missing,
                ..
            }
        ));
    }

    #[test]
    fn load_corrupt_artifact_fails_open() -> Result<()> {
        let dir = test_dir("corrupt");
        let path = dir.join("artifact.json");
        fs::create_dir_all(&dir)?;
        fs::write(&path, "{not-json")?;

        let loaded = load_artifact_fail_open(&path);

        assert!(matches!(
            loaded,
            ArtifactLoad::FailOpen {
                kind: ArtifactLoadErrorKind::Parse,
                ..
            }
        ));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn load_wrong_version_artifact_fails_open() -> Result<()> {
        let dir = test_dir("wrong-version");
        let path = dir.join("artifact.json");
        fs::create_dir_all(&dir)?;
        fs::write(
            &path,
            format!(
                r#"{{"version":{},"compiled_at_epoch":1,"rules":[]}}"#,
                ARTIFACT_VERSION + 1
            ),
        )?;

        let loaded = load_artifact_fail_open(&path);

        assert!(matches!(
            loaded,
            ArtifactLoad::FailOpen {
                kind: ArtifactLoadErrorKind::Validate,
                ..
            }
        ));
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn atomic_writer_preserves_existing_artifact_on_rename_failure() -> Result<()> {
        let dir = test_dir("atomic-failure");
        let path = dir.join("artifact.json");
        let original = CompiledRulesArtifact::new(1, Vec::new());
        write_artifact_atomic(&path, &original)?;
        let replacement = package_manager_artifact();

        let _guard = crate::atomic_file::failpoint_test_lock();
        crate::atomic_file::fail_next_rename_for_path_for_test(&path);
        let err = write_artifact_atomic(&path, &replacement)
            .expect_err("injected rename failure must surface");
        crate::atomic_file::clear_failpoints_for_test();

        assert!(err.to_string().contains("injected atomic write failure"));
        assert_eq!(
            load_artifact_fail_open(&path),
            ArtifactLoad::Loaded(original)
        );
        let temp_entries = fs::read_dir(&dir)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
            .count();
        assert_eq!(temp_entries, 0);
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}
