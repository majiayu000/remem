use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::local_copy::{
    build_local_note_content, local_copy_enabled_override, resolve_local_note_path,
    write_local_note,
};
use super::types::{LocalCopyResult, SaveMemoryNextStep, SaveMemoryRequest, SaveMemoryResult};
use crate::memory::lesson::{save_lesson, SaveLessonRequest};
use crate::memory::lifecycle::MemoryLifecycleOp;

#[derive(Debug)]
pub struct LocalCopyError {
    message: String,
}

impl From<anyhow::Error> for LocalCopyError {
    fn from(err: anyhow::Error) -> Self {
        Self {
            message: err.to_string(),
        }
    }
}

impl std::fmt::Display for LocalCopyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LocalCopyError {}

pub fn save_memory(conn: &Connection, req: &SaveMemoryRequest) -> Result<SaveMemoryResult> {
    let project = req.project.as_deref().unwrap_or("manual");
    let title = req.title.as_deref().unwrap_or("Memory");
    let memory_type = req.memory_type.as_deref().unwrap_or("discovery");
    let files_json = req
        .files
        .as_ref()
        .and_then(|files| serde_json::to_string(files).ok());

    let scope = req.scope.as_deref().unwrap_or("project");
    let effective_topic_key = effective_topic_key(req, memory_type);

    let mut local_copy = prepare_local_copy(project, title, req).map_err(LocalCopyError::from)?;
    write_local_copy(&mut local_copy).map_err(LocalCopyError::from)?;

    let save_result = if memory_type == "lesson" {
        crate::memory::operation::with_operation_savepoint(conn, || {
            let (operation_input, operation_plan) = crate::memory::operation::plan_direct_save(
                conn,
                "direct",
                "save_memory",
                project,
                scope,
                memory_type,
                effective_topic_key.as_deref(),
                title,
                &req.text,
                files_json.as_deref(),
                req.branch.as_deref(),
                None,
                None,
            )?;
            let id = save_lesson(
                conn,
                &SaveLessonRequest {
                    session_id: None,
                    project,
                    topic_key: req.topic_key.as_deref(),
                    title,
                    content: &req.text,
                    confidence: 0.7,
                    source_evidence: None,
                    files: files_json.as_deref(),
                    branch: req.branch.as_deref(),
                    scope,
                    created_at_epoch: req.created_at_epoch,
                    stale_after_epoch: None,
                },
            )?;
            let mut logged_plan = operation_plan.clone();
            logged_plan.target_memory_id = Some(id);
            if logged_plan.op == MemoryLifecycleOp::Noop {
                logged_plan.op = MemoryLifecycleOp::Update;
                logged_plan.noop_reason = None;
                logged_plan.reason =
                    "existing lesson memory was reinforced by direct save".to_string();
            }
            crate::memory::operation::insert_operation_log(
                conn,
                &operation_input,
                &logged_plan,
                Some(id),
            )?;
            Ok((id, logged_plan.op))
        })
    } else {
        crate::memory::operation::with_operation_savepoint(conn, || {
            let (operation_input, operation_plan) = crate::memory::operation::plan_direct_save(
                conn,
                "direct",
                "save_memory",
                project,
                scope,
                memory_type,
                effective_topic_key.as_deref(),
                title,
                &req.text,
                files_json.as_deref(),
                req.branch.as_deref(),
                None,
                None,
            )?;
            if operation_plan.op == MemoryLifecycleOp::Noop {
                let id = operation_plan
                    .target_memory_id
                    .ok_or_else(|| anyhow!("noop memory operation missing existing memory id"))?;
                crate::memory::operation::insert_operation_log(
                    conn,
                    &operation_input,
                    &operation_plan,
                    Some(id),
                )?;
                return Ok((id, MemoryLifecycleOp::Noop));
            }
            crate::memory::insert_memory_full_with_operation_log(
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
                &operation_input,
                &operation_plan,
            )
        })
    };

    let (id, operation) = match save_result {
        Ok(result) => result,
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
    let durable = load_durable_write_details(conn, id)?;
    let local_copy_result = local_copy.result();

    Ok(SaveMemoryResult {
        id,
        status: "saved".to_string(),
        memory_type: durable.memory_type,
        project: durable.project,
        scope: durable.scope,
        topic_key: durable.topic_key,
        branch: durable.branch,
        operation: operation.as_str().to_string(),
        created_at_epoch: durable.created_at_epoch,
        updated_at_epoch: durable.updated_at_epoch,
        upserted: req.topic_key.is_some(),
        local_status: local_copy_result.status.clone(),
        local_path: local_copy_result.path.clone(),
        local_copy: local_copy_result,
        next_step: SaveMemoryNextStep {
            tool: "get_observations".to_string(),
            ids: vec![id],
            source: "memory".to_string(),
            reason: format!(
                "Verify the durable memory with get_observations(ids=[{id}], source='memory') or search(project='{}').",
                durable_project_hint(project)
            ),
        },
    })
}

struct LocalCopyPlan {
    status: String,
    path: Option<PathBuf>,
    reason: Option<String>,
    content: Option<String>,
    backup: Option<LocalCopyBackup>,
}

impl LocalCopyPlan {
    fn result(&self) -> LocalCopyResult {
        LocalCopyResult {
            status: self.status.clone(),
            path: self.path.as_ref().map(|path| path.display().to_string()),
            reason: self.reason.clone(),
        }
    }
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
            reason: Some("local copy disabled by request or configuration".to_string()),
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
        reason: None,
        content: Some(content),
        backup: None,
    })
}

fn effective_topic_key(req: &SaveMemoryRequest, memory_type: &str) -> Option<String> {
    if memory_type == "lesson" {
        return req.topic_key.clone().or_else(|| {
            Some(format!(
                "lesson-{}",
                crate::memory::slugify_for_topic(&req.text, 64)
            ))
        });
    }
    req.topic_key.clone()
}

struct DurableWriteDetails {
    project: String,
    scope: String,
    topic_key: Option<String>,
    branch: Option<String>,
    memory_type: String,
    created_at_epoch: i64,
    updated_at_epoch: i64,
}

fn load_durable_write_details(conn: &Connection, id: i64) -> Result<DurableWriteDetails> {
    conn.query_row(
        "SELECT project, COALESCE(scope, 'project'), topic_key, branch, memory_type,
                created_at_epoch, updated_at_epoch
         FROM memories
         WHERE id = ?1",
        [id],
        |row| {
            Ok(DurableWriteDetails {
                project: row.get(0)?,
                scope: row.get(1)?,
                topic_key: row.get(2)?,
                branch: row.get(3)?,
                memory_type: row.get(4)?,
                created_at_epoch: row.get(5)?,
                updated_at_epoch: row.get(6)?,
            })
        },
    )
    .with_context(|| format!("load durable write details for memory {id}"))
}

fn durable_project_hint(project: &str) -> String {
    project.replace('\'', "\\'")
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
