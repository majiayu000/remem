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

    let scope = req
        .scope
        .as_deref()
        .unwrap_or(if memory_type == "preference" {
            "global"
        } else {
            "project"
        });

    // Validate and resolve the local_path BEFORE the DB insert so that an
    // invalid user-supplied path (e.g. path traversal) is rejected with a
    // clean error before any data is written to the database (U-18).
    let resolved_local_path = if local_copy_enabled_override(req.local_copy_enabled) {
        Some(resolve_local_note_path(
            project,
            req.title.as_deref(),
            req.local_path.as_deref(),
        )?)
    } else {
        None
    };

    // Insert into DB after path validation passes. Writing the local copy
    // before the DB insert would leave an orphaned file on disk if the
    // insert fails (U-17).
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

    // Filesystem write is best-effort: the DB row is the authoritative record.
    // Any I/O error (permissions, ENOSPC, race) is demoted to local_status
    // "failed" so callers always receive the real saved id and do not retry a
    // request that already succeeded in the DB, which would create duplicates.
    let (local_status, local_path) = match resolved_local_path {
        None => ("disabled".to_string(), None),
        Some(path) => {
            let content = build_local_note_content(project, title, &req.text);
            match write_local_note(&path, &content) {
                Ok(()) => ("saved".to_string(), Some(path.display().to_string())),
                // Security violations (path confinement, TOCTOU) must propagate
                // as hard errors — do not demote to local_status="failed".
                Err(e) if e.to_string().contains("outside the allowed directory") => {
                    return Err(e);
                }
                Err(e) => {
                    crate::log::warn(
                        "save",
                        &format!("local copy write failed for id={}: {}", id, e),
                    );
                    ("failed".to_string(), None)
                }
            }
        }
    };

    Ok(SaveMemoryResult {
        id,
        status: "saved".to_string(),
        memory_type: memory_type.to_string(),
        upserted: req.topic_key.is_some(),
        local_status,
        local_path,
    })
}
