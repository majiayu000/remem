use std::{fs::OpenOptions, io::Write, path::Path};

use anyhow::{Context, Result};

use crate::{
    cli::query_types::{ProfileSnapshotFormatArg, UserProfileAction},
    db,
    user_context::profile_snapshot::{render_markdown_profile_snapshot, ProfileSnapshotRequest},
};

use super::shared::resolve_cwd_project;

pub(in crate::cli) fn run_user_profile(action: UserProfileAction) -> Result<()> {
    match action {
        UserProfileAction::Export {
            format,
            output,
            project,
            owner_scope,
            owner_key,
            include_suppressed,
            include_sensitive,
            include_inactive,
            include_deleted,
            include_manual_summaries,
        } => {
            let project = project.unwrap_or_else(|| resolve_cwd_project().1);
            let source_of_truth = db::absolute_data_dir()?.join("remem.db");
            let conn = db::open_db_read_only()?;
            match format {
                ProfileSnapshotFormatArg::Markdown => {
                    let markdown = render_markdown_profile_snapshot(
                        &conn,
                        &ProfileSnapshotRequest {
                            project: &project,
                            owner_scope: owner_scope.db_value(),
                            owner_key: owner_key.as_deref(),
                            source_of_truth: &source_of_truth,
                            include_suppressed,
                            include_sensitive,
                            include_inactive,
                            include_deleted,
                            include_manual_summaries,
                        },
                    )?;
                    write_snapshot_output(&markdown, output.as_deref())?;
                }
            }
        }
    }
    Ok(())
}

fn write_snapshot_output(markdown: &str, output: Option<&Path>) -> Result<()> {
    let Some(path) = output else {
        print!("{markdown}");
        return Ok(());
    };
    write_new_file(path, markdown.as_bytes())
}

fn write_new_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("create profile snapshot {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("write profile snapshot {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_snapshot_output_refuses_to_overwrite_existing_file() -> Result<()> {
        let dir = crate::db::test_support::ScopedTestDataDir::new("profile-output-overwrite");
        std::fs::create_dir_all(&dir.path)?;
        let path = dir.path.join("profile.md");
        std::fs::write(&path, "existing")?;

        let err = write_snapshot_output("new", Some(&path)).expect_err("must not overwrite");
        assert!(err.to_string().contains("create profile snapshot"));
        assert_eq!(std::fs::read_to_string(&path)?, "existing");
        Ok(())
    }

    #[test]
    fn profile_snapshot_output_writes_new_file() -> Result<()> {
        let dir = crate::db::test_support::ScopedTestDataDir::new("profile-output-new");
        std::fs::create_dir_all(&dir.path)?;
        let path: std::path::PathBuf = dir.path.join("profile.md");

        write_snapshot_output("snapshot", Some(&path))?;

        assert_eq!(std::fs::read_to_string(path)?, "snapshot");
        Ok(())
    }
}
