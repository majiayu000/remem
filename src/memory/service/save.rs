use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::local_copy::{
    build_local_note_content, local_copy_enabled_override, resolve_local_note_path,
    write_local_note,
};
use super::types::{LocalCopyResult, SaveMemoryNextStep, SaveMemoryRequest, SaveMemoryResult};
use crate::memory::claims::{claims_enabled, insert_memory_claim, ClaimWriteRequest};
use crate::memory::lesson::{save_lesson_with_reference_time, SaveLessonRequest};
use crate::memory::lifecycle::MemoryLifecycleOp;
use crate::memory::poisoning::{
    scan_instruction_pattern, InstructionPatternMatch, DIRECT_SAVE_TRUST_CLASS,
};
use crate::memory::{MemoryType, MEMORY_TYPES};

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

#[derive(Debug)]
pub struct SaveMemoryValidationError {
    message: String,
}

impl SaveMemoryValidationError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SaveMemoryValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SaveMemoryValidationError {}

pub fn save_memory(conn: &Connection, req: &SaveMemoryRequest) -> Result<SaveMemoryResult> {
    save_memory_with_reference_time(conn, req, req.created_at_epoch)
}

pub fn save_memory_with_reference_time(
    conn: &Connection,
    req: &SaveMemoryRequest,
    reference_time_epoch: Option<i64>,
) -> Result<SaveMemoryResult> {
    let reference_time_epoch =
        normalize_reference_time_epoch(req.created_at_epoch, reference_time_epoch);
    save_memory_inner(conn, req, reference_time_epoch)
}

fn normalize_reference_time_epoch(
    created_at_epoch: Option<i64>,
    reference_time_epoch: Option<i64>,
) -> Option<i64> {
    reference_time_epoch.or(created_at_epoch)
}

fn save_memory_inner(
    conn: &Connection,
    req: &SaveMemoryRequest,
    reference_time_epoch: Option<i64>,
) -> Result<SaveMemoryResult> {
    let validated = validate_save_memory_request(req)?;
    let project = req.project.as_deref().unwrap_or("manual");
    let title = req.title.as_deref().unwrap_or("Memory");
    let memory_type = validated.memory_type.as_str();
    let files_json = req
        .files
        .as_ref()
        .and_then(|files| serde_json::to_string(files).ok());

    let scope = validated.scope.as_str();
    let effective_topic_key = effective_topic_key(req, memory_type);
    let acknowledgement =
        direct_save_pattern_acknowledgement(title, &req.text, req.acknowledge_pattern.as_deref())?;

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
            let id = save_lesson_with_reference_time(
                conn,
                &SaveLessonRequest {
                    session_id: req.session_id.as_deref(),
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
                reference_time_epoch,
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
            mark_direct_save_poisoning_metadata(conn, id, acknowledgement)?;
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
                mark_direct_save_poisoning_metadata(conn, id, acknowledgement)?;
                return Ok((id, MemoryLifecycleOp::Noop));
            }
            let result = crate::memory::insert_memory_full_with_operation_log(
                conn,
                req.session_id.as_deref(),
                project,
                req.topic_key.as_deref(),
                title,
                &req.text,
                memory_type,
                files_json.as_deref(),
                req.branch.as_deref(),
                scope,
                req.created_at_epoch,
                reference_time_epoch,
                &operation_input,
                &operation_plan,
            )?;
            mark_direct_save_poisoning_metadata(conn, result.0, acknowledgement)?;
            Ok(result)
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
    let claim_result = write_claim_after_durable_save(conn, id, req);

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
        reference_time_epoch: durable.reference_time_epoch,
        updated_at_epoch: durable.updated_at_epoch,
        upserted: req.topic_key.is_some(),
        local_status: local_copy_result.status.clone(),
        local_path: local_copy_result.path.clone(),
        local_copy: local_copy_result,
        claim_status: claim_result.status,
        claim_id: claim_result.id,
        claim_error: claim_result.error,
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

fn direct_save_pattern_acknowledgement(
    title: &str,
    text: &str,
    acknowledged_pattern_id: Option<&str>,
) -> Result<Option<InstructionPatternMatch>> {
    let scan_text = format!("{title}\n{text}");
    let matched = scan_instruction_pattern(&scan_text);
    let acknowledged_pattern_id = acknowledged_pattern_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (matched, acknowledged_pattern_id) {
        (Some(matched), Some(acknowledged)) if acknowledged == matched.pattern_id => {
            Ok(Some(matched))
        }
        (Some(matched), Some(acknowledged)) => {
            Err(SaveMemoryValidationError::new(format!(
                "save_memory acknowledged pattern {acknowledged} does not match instruction-pattern {}@v{}",
                matched.pattern_id, matched.pattern_set_version
            ))
            .into())
        }
        (Some(matched), None) => Err(SaveMemoryValidationError::new(format!(
            "save_memory text matched instruction-pattern {}@v{}; review and acknowledge the pattern before saving",
            matched.pattern_id, matched.pattern_set_version
        ))
        .into()),
        (None, Some(acknowledged)) => Err(SaveMemoryValidationError::new(format!(
            "save_memory acknowledge_pattern {acknowledged} was provided, but no instruction-pattern matched"
        ))
        .into()),
        (None, None) => Ok(None),
    }
}

fn mark_direct_save_poisoning_metadata(
    conn: &Connection,
    memory_id: i64,
    acknowledgement: Option<InstructionPatternMatch>,
) -> Result<()> {
    if let Some(acknowledgement) = acknowledgement {
        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "UPDATE memories
             SET source_trust_class = ?1,
                 acknowledged_pattern_id = ?2,
                 acknowledged_pattern_version = ?3,
                 acknowledged_at_epoch = ?4
             WHERE id = ?5",
            rusqlite::params![
                DIRECT_SAVE_TRUST_CLASS.as_str(),
                acknowledgement.pattern_id,
                acknowledgement.pattern_set_version,
                now,
                memory_id
            ],
        )?;
    } else {
        conn.execute(
            "UPDATE memories SET source_trust_class = ?1 WHERE id = ?2",
            rusqlite::params![DIRECT_SAVE_TRUST_CLASS.as_str(), memory_id],
        )?;
    }
    Ok(())
}

struct ValidatedSaveMemoryRequest {
    memory_type: String,
    scope: String,
}

fn validate_save_memory_request(req: &SaveMemoryRequest) -> Result<ValidatedSaveMemoryRequest> {
    if req.text.trim().is_empty() {
        return Err(SaveMemoryValidationError::new("save_memory text must not be blank").into());
    }

    let memory_type = match req.memory_type.as_deref() {
        Some(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            let parsed = MemoryType::parse(&normalized).ok_or_else(|| {
                SaveMemoryValidationError::new(format!(
                    "save_memory memory_type must be one of: {}",
                    MEMORY_TYPES.join(", ")
                ))
            })?;
            parsed.as_str().to_string()
        }
        None => MemoryType::Discovery.as_str().to_string(),
    };

    let scope = match req.scope.as_deref() {
        Some(value) => match value.trim().to_ascii_lowercase().as_str() {
            "project" => "project".to_string(),
            "global" => "global".to_string(),
            _ => {
                return Err(SaveMemoryValidationError::new(
                    "save_memory scope must be one of: project, global",
                )
                .into());
            }
        },
        None => "project".to_string(),
    };

    Ok(ValidatedSaveMemoryRequest { memory_type, scope })
}

struct ClaimSaveResult {
    status: String,
    id: Option<i64>,
    error: Option<String>,
}

fn write_claim_after_durable_save(
    conn: &Connection,
    memory_id: i64,
    req: &SaveMemoryRequest,
) -> ClaimSaveResult {
    if !claims_enabled(req.claim_enabled) {
        return ClaimSaveResult {
            status: "disabled".to_string(),
            id: None,
            error: None,
        };
    }

    let claim_source = req.claim_source.as_deref().unwrap_or("manual_save");
    match insert_memory_claim(
        conn,
        &ClaimWriteRequest {
            memory_id,
            session_id: req.session_id.as_deref(),
            host: req.host.as_deref(),
            claim_source,
        },
    ) {
        Ok(claim_id) => ClaimSaveResult {
            status: "saved".to_string(),
            id: Some(claim_id),
            error: None,
        },
        Err(err) => {
            let error = format!("{err:#}");
            crate::log::error(
                "memory-claim",
                &format!("claim write failed memory_id={} error={}", memory_id, error),
            );
            ClaimSaveResult {
                status: "failed".to_string(),
                id: None,
                error: Some(error),
            }
        }
    }
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
    reference_time_epoch: i64,
    updated_at_epoch: i64,
}

fn load_durable_write_details(conn: &Connection, id: i64) -> Result<DurableWriteDetails> {
    conn.query_row(
        "SELECT project, COALESCE(scope, 'project'), topic_key, branch, memory_type,
                created_at_epoch, COALESCE(reference_time_epoch, created_at_epoch), updated_at_epoch
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
                reference_time_epoch: row.get(6)?,
                updated_at_epoch: row.get(7)?,
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
