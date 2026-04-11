use anyhow::{anyhow, Result};
use chrono::Utc;
use std::path::{Component, Path, PathBuf};

const LOCAL_SAVE_ENABLE_ENV: &str = "REMEM_SAVE_MEMORY_LOCAL_COPY";
const LOCAL_SAVE_DIR_ENV: &str = "REMEM_SAVE_MEMORY_LOCAL_DIR";

pub(super) fn local_copy_enabled_override(requested: Option<bool>) -> bool {
    requested.unwrap_or_else(|| env_enabled(LOCAL_SAVE_ENABLE_ENV, true))
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

/// Resolves `local_path` to an absolute path confined within the allowed base
/// directory (`remem_data_dir()`).  Returns `Err` if the resolved path escapes
/// the base or is equal to the base itself.  When `local_path` is `None` or
/// empty the default note path is returned (always inside the base).
pub fn resolve_local_note_path(
    project: &str,
    title: Option<&str>,
    local_path: Option<&str>,
) -> Result<PathBuf> {
    if let Some(raw) = local_path.and_then(non_empty_trimmed) {
        let path = PathBuf::from(raw);
        let abs = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };
        confine_to_base(&abs)
    } else {
        Ok(default_local_note_path(project, title))
    }
}

/// Normalises `..` and `.` components without calling `canonicalize` (which
/// fails for paths that do not yet exist).
fn normalize_path(path: &Path) -> PathBuf {
    let mut out: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if matches!(out.last(), Some(Component::Normal(_))) {
                    out.pop();
                }
                // At root or already at top — just discard the `..`
            }
            Component::CurDir => {}
            _ => out.push(component),
        }
    }
    out.iter().collect()
}

/// Checks that `abs` (an absolute path) is strictly inside `remem_data_dir()`.
/// If the parent directory already exists it is canonicalized to resolve
/// symlinks before the prefix check, preventing symlink-based escapes.
fn confine_to_base(abs: &Path) -> Result<PathBuf> {
    let raw_base = remem_data_dir();
    // Canonicalize base if it exists so the prefix check is symlink-safe.
    let base = if raw_base.exists() {
        raw_base.canonicalize().unwrap_or(raw_base)
    } else {
        raw_base
    };

    let normalized = normalize_path(abs);

    // Canonicalize the parent directory (likely exists) to detect symlinks.
    let resolved = if let Some(parent) = normalized.parent() {
        if parent.exists() {
            match parent.canonicalize() {
                Ok(canon_parent) => canon_parent.join(normalized.file_name().unwrap_or_default()),
                Err(_) => normalized.clone(),
            }
        } else {
            normalized.clone()
        }
    } else {
        normalized.clone()
    };

    if !resolved.starts_with(&base) || resolved == base {
        return Err(anyhow!("local_path is outside the allowed directory"));
    }
    Ok(resolved)
}

pub(super) fn build_local_note_content(project: &str, title: &str, text: &str) -> String {
    let now = Utc::now().to_rfc3339();
    format!(
        "---\nsource: remem.save_memory\nsaved_at: {}\nproject: {}\n---\n\n# {}\n\n{}\n",
        now, project, title, text
    )
}

pub(super) fn write_local_note(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

fn env_enabled(key: &str, default: bool) -> bool {
    match std::env::var(key) {
        Ok(value) => {
            let lower = value.trim().to_ascii_lowercase();
            !matches!(lower.as_str(), "0" | "false" | "no" | "off")
        }
        Err(_) => default,
    }
}

fn remem_data_dir() -> PathBuf {
    crate::db::data_dir()
}

fn default_local_note_path(project: &str, title: Option<&str>) -> PathBuf {
    let base = std::env::var(LOCAL_SAVE_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| remem_data_dir().join("manual-notes"));
    let project_dir = sanitize_segment(project, "manual", 64);
    let title_slug = sanitize_segment(title.unwrap_or("memory"), "memory", 64);
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    base.join(project_dir)
        .join(format!("{}-{}.md", timestamp, title_slug))
}

fn non_empty_trimmed(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
