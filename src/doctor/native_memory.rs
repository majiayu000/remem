use std::path::{Path, PathBuf};

use super::types::{Check, Status};

const MAX_REPORTED_FILES: usize = 3;

pub(super) fn check_native_memory_sync() -> Check {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let projects_dir = home.join(".claude").join("projects");
    check_native_memory_sync_for(
        &projects_dir,
        crate::context::claude_memory::native_memory_max_bytes(),
        crate::context::claude_memory::native_memory_sync_disabled(),
    )
}

fn check_native_memory_sync_for(projects_dir: &Path, max_bytes: usize, disabled: bool) -> Check {
    let mut files = match collect_remem_native_memory_files(projects_dir) {
        Ok(files) => files,
        Err(err) => {
            return Check {
                name: "Claude native memory",
                status: Status::Warn,
                detail: format!("cannot inspect {}: {}", projects_dir.display(), err),
            };
        }
    };

    if files.is_empty() {
        return Check {
            name: "Claude native memory",
            status: Status::Ok,
            detail: if disabled {
                format!(
                    "disabled by {}; no remem_sessions.md files found",
                    crate::context::claude_memory::DISABLE_NATIVE_MEMORY_SYNC_ENV
                )
            } else {
                "no remem_sessions.md files found".to_string()
            },
        };
    }

    files.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    let oversized = files
        .iter()
        .filter(|file| file.bytes > max_bytes as u64)
        .collect::<Vec<_>>();
    let largest = files.first().map(|file| file.bytes).unwrap_or(0);
    let disabled_note = if disabled {
        format!(
            "; future sync disabled by {}",
            crate::context::claude_memory::DISABLE_NATIVE_MEMORY_SYNC_ENV
        )
    } else {
        String::new()
    };

    if oversized.is_empty() {
        return Check {
            name: "Claude native memory",
            status: Status::Ok,
            detail: format!(
                "{} remem_sessions.md file(s), largest {} bytes, limit {}={} bytes{}",
                files.len(),
                largest,
                crate::context::claude_memory::NATIVE_MEMORY_MAX_BYTES_ENV,
                max_bytes,
                disabled_note
            ),
        };
    }

    let examples = oversized
        .iter()
        .take(MAX_REPORTED_FILES)
        .map(|file| format!("{} ({} bytes)", file.path.display(), file.bytes))
        .collect::<Vec<_>>()
        .join("; ");
    Check {
        name: "Claude native memory",
        status: Status::Warn,
        detail: format!(
            "{} of {} remem_sessions.md file(s) exceed {} bytes{}; host-side Claude Code loads are not included in remem usage: {}",
            oversized.len(),
            files.len(),
            max_bytes,
            disabled_note,
            examples
        ),
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NativeMemoryFile {
    path: PathBuf,
    bytes: u64,
}

fn collect_remem_native_memory_files(
    projects_dir: &Path,
) -> std::io::Result<Vec<NativeMemoryFile>> {
    if !projects_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in std::fs::read_dir(projects_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        let file_path = entry
            .path()
            .join("memory")
            .join(crate::context::claude_memory::REMEM_FILE);
        if file_path.exists() {
            let bytes = std::fs::metadata(&file_path)?.len();
            files.push(NativeMemoryFile {
                path: file_path,
                bytes,
            });
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doctor_temp_dir(name: &str) -> anyhow::Result<PathBuf> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "remem-doctor-{name}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn native_memory_check_warns_for_oversized_remem_sessions() -> anyhow::Result<()> {
        let projects_dir = doctor_temp_dir("native-memory")?;
        let memory_dir = projects_dir.join("project-a").join("memory");
        std::fs::create_dir_all(&memory_dir)?;
        std::fs::write(
            memory_dir.join(crate::context::claude_memory::REMEM_FILE),
            "x".repeat(32),
        )?;

        let check = check_native_memory_sync_for(&projects_dir, 16, false);

        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("exceed 16 bytes"), "{}", check.detail);
        assert!(
            check.detail.contains("not included in remem usage"),
            "{}",
            check.detail
        );

        std::fs::remove_dir_all(projects_dir)?;
        Ok(())
    }

    #[test]
    fn native_memory_check_reports_disabled_state_without_files() -> anyhow::Result<()> {
        let projects_dir = doctor_temp_dir("native-memory-disabled")?;

        let check = check_native_memory_sync_for(&projects_dir, 16, true);

        assert!(matches!(check.status, Status::Ok));
        assert!(
            check
                .detail
                .contains(crate::context::claude_memory::DISABLE_NATIVE_MEMORY_SYNC_ENV),
            "{}",
            check.detail
        );

        std::fs::remove_dir_all(projects_dir)?;
        Ok(())
    }
}
