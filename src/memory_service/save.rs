use anyhow::Result;
use rusqlite::Connection;

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

    let (local_status, local_path) = maybe_write_local_copy(project, title, req)?;
    let scope = req
        .scope
        .as_deref()
        .unwrap_or(if memory_type == "preference" {
            "global"
        } else {
            "project"
        });
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

    Ok(SaveMemoryResult {
        id,
        status: "saved".to_string(),
        memory_type: memory_type.to_string(),
        upserted: req.topic_key.is_some(),
        local_status,
        local_path,
    })
}

fn maybe_write_local_copy(
    project: &str,
    title: &str,
    req: &SaveMemoryRequest,
) -> Result<(String, Option<String>)> {
    if !local_copy_enabled_override(req.local_copy_enabled) {
        return Ok(("disabled".to_string(), None));
    }

    let local_path =
        resolve_local_note_path(project, req.title.as_deref(), req.local_path.as_deref());
    let content = build_local_note_content(project, title, &req.text);
    write_local_note(&local_path, &content)?;
    Ok(("saved".to_string(), Some(local_path.display().to_string())))
}
