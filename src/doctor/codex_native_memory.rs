//! Doctor check for Codex native memories (GH-852 B-011, B-014): reports
//! not_configured / ready / unreadable / unsupported_format without printing
//! memory bodies or secrets.

use std::fs;
use std::path::Path;

use super::types::{Check, Status};

const CHECK_NAME: &str = "Codex native memories";

pub(super) fn check_codex_native_memories() -> Check {
    let source_dir = crate::install::codex_memories_dir();
    check_codex_native_memories_for(&source_dir)
}

fn check_codex_native_memories_for(source_dir: &Path) -> Check {
    match fs::symlink_metadata(source_dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Check::new(
                CHECK_NAME,
                Status::Ok,
                "not_configured: no Codex native memories directory".to_string(),
            );
        }
        Err(err) => {
            return Check::new(
                CHECK_NAME,
                Status::Warn,
                format!("unreadable: cannot stat source directory: {err}"),
            );
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Check::new(
                CHECK_NAME,
                Status::Warn,
                "unreadable: source path exists but is not a directory".to_string(),
            );
        }
        Ok(_) => {}
    }

    let entries = match fs::read_dir(source_dir) {
        Ok(entries) => entries,
        Err(err) => {
            return Check::new(
                CHECK_NAME,
                Status::Warn,
                format!("unreadable: cannot list source directory: {err}"),
            );
        }
    };

    let mut recognized = 0usize;
    let mut unsupported = 0usize;
    for entry in entries {
        let Ok(entry) = entry else {
            return Check::new(
                CHECK_NAME,
                Status::Warn,
                "unreadable: failed to enumerate a source entry".to_string(),
            );
        };
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_file = entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false);
        if is_file && crate::install::is_codex_rollout_summary_filename(&name) {
            recognized += 1;
        } else {
            unsupported += 1;
        }
    }

    if unsupported > 0 {
        return Check::new(
            CHECK_NAME,
            Status::Warn,
            format!(
                "unsupported_format: {unsupported} entr(ies) do not match codex-rollout-summary/v1 \
                 ({recognized} recognized); import will fail closed until resolved"
            ),
        );
    }
    Check::new(
        CHECK_NAME,
        Status::Ok,
        format!(
            "ready: {recognized} codex-rollout-summary/v1 record(s); import via \
             `remem import codex-memories --dry-run`"
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "remem-doctor-codex-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn missing_directory_is_not_configured() {
        let check = check_codex_native_memories_for(Path::new("/nonexistent/remem-codex-memories"));
        assert!(matches!(check.status, Status::Ok));
        assert!(
            check.detail.starts_with("not_configured"),
            "{}",
            check.detail
        );
    }

    #[test]
    fn recognized_records_report_ready() {
        let dir = temp_dir("ready");
        std::fs::write(
            dir.join("2026-07-01T10-00-00-abCD-sample_topic.md"),
            "thread_id: t\n",
        )
        .expect("write fixture");
        let check = check_codex_native_memories_for(&dir);
        assert!(matches!(check.status, Status::Ok));
        assert!(check.detail.starts_with("ready: 1"), "{}", check.detail);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn unknown_entries_report_unsupported_format() {
        let dir = temp_dir("unsupported");
        std::fs::write(dir.join("notes.txt"), "x").expect("write fixture");
        let check = check_codex_native_memories_for(&dir);
        assert!(matches!(check.status, Status::Warn));
        assert!(
            check.detail.starts_with("unsupported_format"),
            "{}",
            check.detail
        );
        std::fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn not_a_directory_reports_unreadable() {
        let dir = temp_dir("notdir");
        let file = dir.join("plain");
        std::fs::write(&file, "x").expect("write fixture");
        let check = check_codex_native_memories_for(&file);
        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.starts_with("unreadable"), "{}", check.detail);
        std::fs::remove_dir_all(dir).expect("cleanup");
    }
}
