use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use crate::{memory, search};

const LOCAL_SAVE_ENABLE_ENV: &str = "REMEM_SAVE_MEMORY_LOCAL_COPY";
const LOCAL_SAVE_DIR_ENV: &str = "REMEM_SAVE_MEMORY_LOCAL_DIR";

#[derive(Debug, Clone, Default)]
pub struct SearchRequest {
    pub query: Option<String>,
    pub project: Option<String>,
    pub memory_type: Option<String>,
    pub limit: i64,
    pub offset: i64,
    pub include_stale: bool,
    pub branch: Option<String>,
    pub multi_hop: bool,
}

#[derive(Debug, Clone)]
pub struct MultiHopMeta {
    pub hops: u8,
    pub entities_discovered: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchResultSet {
    pub memories: Vec<memory::Memory>,
    pub multi_hop: Option<MultiHopMeta>,
}

#[derive(Debug, Clone, Default)]
pub struct SaveMemoryRequest {
    pub text: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub topic_key: Option<String>,
    pub memory_type: Option<String>,
    pub files: Option<Vec<String>>,
    pub scope: Option<String>,
    pub created_at_epoch: Option<i64>,
    pub branch: Option<String>,
    pub local_path: Option<String>,
    pub local_copy_enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct SaveMemoryResult {
    pub id: i64,
    pub status: String,
    pub memory_type: String,
    pub upserted: bool,
    pub local_status: String,
    pub local_path: Option<String>,
}

fn env_enabled(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(v) => {
            let lower = v.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => default,
    }
}

fn remem_data_dir() -> PathBuf {
    crate::db::data_dir()
}

pub fn sanitize_segment(raw: &str, fallback: &str, limit: usize) -> String {
    let mut out = String::with_capacity(raw.len().min(limit));
    let mut last_underscore = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() || ch == '_' {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' {
            if last_underscore {
                continue;
            }
            last_underscore = true;
        } else {
            last_underscore = false;
        }
        out.push(mapped);
        if out.len() >= limit {
            break;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn default_local_note_path(project: &str, title: Option<&str>) -> PathBuf {
    let base = std::env::var(LOCAL_SAVE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| remem_data_dir().join("manual-notes"));
    let project_dir = sanitize_segment(project, "manual", 64);
    let title_slug = sanitize_segment(title.unwrap_or("memory"), "memory", 64);
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    base.join(project_dir)
        .join(format!("{}-{}.md", ts, title_slug))
}

pub fn resolve_local_note_path(
    project: &str,
    title: Option<&str>,
    local_path: Option<&str>,
) -> PathBuf {
    if let Some(raw) = local_path.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    }) {
        let p = PathBuf::from(raw);
        if p.is_absolute() {
            p
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(p)
        }
    } else {
        default_local_note_path(project, title)
    }
}

fn build_local_note_content(project: &str, title: &str, text: &str) -> String {
    let now = Utc::now().to_rfc3339();
    format!(
        "---\nsource: remem.save_memory\nsaved_at: {}\nproject: {}\n---\n\n# {}\n\n{}\n",
        now, project, title, text
    )
}

fn write_local_note(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

pub fn search_memories(conn: &Connection, req: &SearchRequest) -> Result<SearchResultSet> {
    let limit = req.limit.max(1);
    let query = req.query.as_deref();
    let auto_multi_hop = query.is_some_and(|q| crate::entity::extract_entities(q, "").len() >= 2);
    let multi_hop = req.multi_hop || auto_multi_hop;

    if multi_hop {
        if let Some(q) = query.filter(|q| !q.is_empty()) {
            let mh =
                crate::search_multihop::search_multi_hop(conn, q, req.project.as_deref(), limit)?;
            Ok(SearchResultSet {
                memories: mh.memories,
                multi_hop: Some(MultiHopMeta {
                    hops: mh.hops,
                    entities_discovered: mh.entities_discovered,
                }),
            })
        } else {
            Ok(SearchResultSet {
                memories: vec![],
                multi_hop: Some(MultiHopMeta {
                    hops: 1,
                    entities_discovered: vec![],
                }),
            })
        }
    } else {
        let memories = search::search_with_branch(
            conn,
            query,
            req.project.as_deref(),
            req.memory_type.as_deref(),
            limit,
            req.offset.max(0),
            req.include_stale,
            req.branch.as_deref(),
        )?;
        Ok(SearchResultSet {
            memories,
            multi_hop: None,
        })
    }
}

pub fn save_memory(conn: &Connection, req: &SaveMemoryRequest) -> Result<SaveMemoryResult> {
    let project = req.project.as_deref().unwrap_or("manual");
    let title = req.title.as_deref().unwrap_or("Memory");
    let memory_type = req.memory_type.as_deref().unwrap_or("discovery");
    let files_json = req
        .files
        .as_ref()
        .and_then(|f| serde_json::to_string(f).ok());

    let local_copy_enabled = req
        .local_copy_enabled
        .unwrap_or_else(|| env_enabled(LOCAL_SAVE_ENABLE_ENV, true));

    let mut local_path_str: Option<String> = None;
    let local_status = if local_copy_enabled {
        let local_path =
            resolve_local_note_path(project, req.title.as_deref(), req.local_path.as_deref());
        let content = build_local_note_content(project, title, &req.text);
        write_local_note(&local_path, &content)?;
        local_path_str = Some(local_path.display().to_string());
        "saved".to_string()
    } else {
        "disabled".to_string()
    };

    let scope = req
        .scope
        .as_deref()
        .unwrap_or(if memory_type == "preference" {
            "global"
        } else {
            "project"
        });
    let id = memory::insert_memory_full(
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
        local_path: local_path_str,
    })
}
