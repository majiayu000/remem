use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::local_copy::{
    build_local_note_content, local_copy_enabled_override, resolve_local_note_path,
    write_local_note,
};
use super::types::{SaveMemoryRequest, SaveMemoryResult};

pub fn save_memory(conn: &Connection, req: &SaveMemoryRequest) -> Result<SaveMemoryResult> {
    let project = req.project.as_deref().unwrap_or("manual");
    let title = req.title.as_deref().unwrap_or("Memory");
    let memory_type = req.memory_type.as_deref().unwrap_or("discovery");
    let files_json = req
        .files
        .as_ref()
        .and_then(|files| serde_json::to_string(files).ok());

    let scope = req
        .scope
        .as_deref()
        .unwrap_or(if memory_type == "preference" {
            "global"
        } else {
            "project"
        });
    let mut local_copy = prepare_local_copy(project, title, req)?;
    write_local_copy(&mut local_copy)?;

    let id = match crate::memory::insert_memory_full(
        conn,
        None,
        project,
        req.topic_key.as_deref(),
        title,
        &req.text,
        memory_type,
        files_json.as_deref(),
        req.branch.as_deref(),
        scope,
        req.created_at_epoch,
    ) {
        Ok(id) => id,
        Err(err) => {
            if let Err(cleanup_err) = cleanup_local_copy(&local_copy) {
                return Err(err.context(format!(
                    "database save failed and local copy cleanup failed: {cleanup_err}"
                )));
            }
            return Err(err);
        }
    };

    discard_local_copy_backup(&local_copy);

    Ok(SaveMemoryResult {
        id,
        status: "saved".to_string(),
        memory_type: memory_type.to_string(),
        upserted: req.topic_key.is_some(),
        local_status: local_copy.status.clone(),
        local_path: local_copy
            .path
            .as_ref()
            .map(|path| path.display().to_string()),
    })
}

struct LocalCopyPlan {
    status: String,
    path: Option<PathBuf>,
    content: Option<String>,
    backup: Option<LocalCopyBackup>,
}

struct LocalCopyBackup {
    restore_path: PathBuf,
    backup_path: PathBuf,
}

fn prepare_local_copy(
    project: &str,
    title: &str,
    req: &SaveMemoryRequest,
) -> Result<LocalCopyPlan> {
    if !local_copy_enabled_override(req.local_copy_enabled) {
        return Ok(LocalCopyPlan {
            status: "disabled".to_string(),
            path: None,
            content: None,
            backup: None,
        });
    }

    let local_path =
        resolve_local_note_path(project, req.title.as_deref(), req.local_path.as_deref())?;
    let content = build_local_note_content(project, title, &req.text);
    Ok(LocalCopyPlan {
        status: "saved".to_string(),
        path: Some(local_path),
        content: Some(content),
        backup: None,
    })
}

fn write_local_copy(local_copy: &mut LocalCopyPlan) -> Result<()> {
    if let (Some(path), Some(content)) = (local_copy.path.as_deref(), local_copy.content.as_deref())
    {
        let backup = backup_existing_local_copy(path)?;
        if let Err(err) = write_local_note(path, content) {
            if let Err(restore_err) = restore_local_copy(backup.as_ref()) {
                return Err(err.context(format!(
                    "write local copy failed and restore failed: {restore_err}"
                )));
            }
            return Err(err);
        }
        local_copy.backup = backup;
    }
    Ok(())
}

fn cleanup_local_copy(local_copy: &LocalCopyPlan) -> Result<()> {
    restore_local_copy(local_copy.backup.as_ref())?;
    match (local_copy.path.as_deref(), local_copy.backup.as_ref()) {
        (Some(path), None) => remove_local_copy_file(path),
        _ => Ok(()),
    }
}

fn backup_existing_local_copy(path: &Path) -> Result<Option<LocalCopyBackup>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            let restore_path = backup_restore_path(path, &metadata)?;
            let backup_path = allocate_backup_path(&restore_path);
            std::fs::rename(&restore_path, &backup_path).with_context(|| {
                format!(
                    "move existing local copy {} to backup {}",
                    restore_path.display(),
                    backup_path.display()
                )
            })?;
            Ok(Some(LocalCopyBackup {
                restore_path,
                backup_path,
            }))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(anyhow!(
            "check existing local copy at {}: {err}",
            path.display()
        )),
    }
}

fn backup_restore_path(path: &Path, metadata: &std::fs::Metadata) -> Result<PathBuf> {
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        return Err(anyhow!(
            "local_path {} must reference a file, not a directory",
            path.display()
        ));
    }

    if file_type.is_symlink() {
        let target_path = path
            .canonicalize()
            .with_context(|| format!("resolve local_path symlink target at {}", path.display()))?;
        if target_path.is_dir() {
            return Err(anyhow!(
                "local_path {} must reference a file, not a directory",
                path.display()
            ));
        }
        return Ok(target_path);
    }

    Ok(path.to_path_buf())
}

fn restore_local_copy(backup: Option<&LocalCopyBackup>) -> Result<()> {
    if let Some(backup) = backup {
        remove_local_copy_file(&backup.restore_path)?;
        std::fs::rename(&backup.backup_path, &backup.restore_path).with_context(|| {
            format!(
                "restore local copy from backup {} to {}",
                backup.backup_path.display(),
                backup.restore_path.display()
            )
        })?;
    }
    Ok(())
}

fn remove_local_copy_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("remove local copy at {}", path.display())),
    }
}

fn discard_local_copy_backup(local_copy: &LocalCopyPlan) {
    if let Some(backup) = local_copy.backup.as_ref() {
        let _ = std::fs::remove_file(&backup.backup_path);
    }
}

fn allocate_backup_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("local-copy");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    parent.join(format!(
        ".{file_name}.remem-backup-{}-{timestamp}.tmp",
        std::process::id()
    ))
}
