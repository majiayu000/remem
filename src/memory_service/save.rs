use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::PathBuf;

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
    let local_copy = prepare_local_copy(project, title, req)?;

    conn.execute_batch("BEGIN IMMEDIATE")
        .context("begin save_memory transaction")?;

    let result = (|| -> Result<SaveMemoryResult> {
        let id = crate::memory::insert_memory_full(
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
        )?;
        write_local_copy(&local_copy)?;
        conn.execute_batch("COMMIT")
            .context("commit save_memory transaction")?;

        Ok(SaveMemoryResult {
            id,
            status: "saved".to_string(),
            memory_type: memory_type.to_string(),
            upserted: req.topic_key.is_some(),
            local_status: local_copy.status.clone(),
            local_path: local_copy.path.as_ref().map(|path| path.display().to_string()),
        })
    })();

    match result {
        Ok(saved) => Ok(saved),
        Err(err) => {
            conn.execute_batch("ROLLBACK")
                .context("rollback save_memory transaction")?;
            Err(err)
        }
    }
}

struct LocalCopyPlan {
    status: String,
    path: Option<PathBuf>,
    content: Option<String>,
}

fn prepare_local_copy(project: &str, title: &str, req: &SaveMemoryRequest) -> Result<LocalCopyPlan> {
    if !local_copy_enabled_override(req.local_copy_enabled) {
        return Ok(LocalCopyPlan {
            status: "disabled".to_string(),
            path: None,
            content: None,
        });
    }

    let local_path =
        resolve_local_note_path(project, req.title.as_deref(), req.local_path.as_deref())?;
    let content = build_local_note_content(project, title, &req.text);
    Ok(LocalCopyPlan {
        status: "saved".to_string(),
        path: Some(local_path),
        content: Some(content),
    })
}

fn write_local_copy(local_copy: &LocalCopyPlan) -> Result<()> {
    if let (Some(path), Some(content)) = (local_copy.path.as_deref(), local_copy.content.as_deref()) {
        write_local_note(path, content)?;
    }
    Ok(())
}
