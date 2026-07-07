use anyhow::Result;
use rusqlite::params;

use crate::context::claude_memory::index::ensure_memory_index;
use crate::context::claude_memory::paths::claude_memory_dir;
use crate::context::claude_memory::render::{
    max_sessions, render_memory_content, SessionRow, REMEM_FILE,
};

pub(crate) const DISABLE_NATIVE_MEMORY_SYNC_ENV: &str = "REMEM_DISABLE_NATIVE_MEMORY_SYNC";
pub(crate) const NATIVE_MEMORY_MAX_BYTES_ENV: &str = "REMEM_NATIVE_MEMORY_MAX_BYTES";

const DEFAULT_NATIVE_MEMORY_MAX_BYTES: usize = 16 * 1024;
const TRUNCATION_NOTE: &str =
    "\n\n---\n*Truncated by remem native memory guard; use `remem search` for full memory.*\n";

pub fn sync_to_claude_memory(conn: &rusqlite::Connection, cwd: &str, project: &str) -> Result<()> {
    if native_memory_sync_disabled() {
        crate::log::info(
            "claude-mem",
            &format!(
                "skip: native memory sync disabled by {} for {}",
                DISABLE_NATIVE_MEMORY_SYNC_ENV, project
            ),
        );
        return Ok(());
    }

    let memory_dir = claude_memory_dir(cwd);

    if !memory_dir.exists() {
        crate::log::info(
            "claude-mem",
            &format!("skip: no claude memory dir for {}", project),
        );
        return Ok(());
    }

    let sessions = load_recent_sessions(conn, project)?;
    let Some(content) = render_memory_content(&sessions) else {
        return Ok(());
    };
    let original_bytes = content.len();
    let max_bytes = native_memory_max_bytes();
    let (content, truncated) = enforce_native_memory_limit(&content, max_bytes);

    let file_path = memory_dir.join(REMEM_FILE);
    std::fs::write(&file_path, &content)?;
    ensure_memory_index(&memory_dir)?;

    let decisions_count = content.matches("\n- ").count();
    crate::log::info(
        "claude-mem",
        &format!(
            "synced {} sessions + {} decisions to {} ({} bytes{})",
            sessions.len(),
            decisions_count,
            file_path.display(),
            content.len(),
            if truncated {
                " after native memory size cap"
            } else {
                ""
            }
        ),
    );
    if truncated {
        crate::log::warn(
            "claude-mem",
            &format!(
                "native memory output for {} capped from {} to {} bytes by {}={}",
                project,
                original_bytes,
                content.len(),
                NATIVE_MEMORY_MAX_BYTES_ENV,
                max_bytes
            ),
        );
    }

    Ok(())
}

pub(crate) fn native_memory_sync_disabled() -> bool {
    std::env::var(DISABLE_NATIVE_MEMORY_SYNC_ENV)
        .ok()
        .is_some_and(|value| env_value_enabled(&value))
}

pub(super) fn env_value_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) fn native_memory_max_bytes() -> usize {
    std::env::var(NATIVE_MEMORY_MAX_BYTES_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_NATIVE_MEMORY_MAX_BYTES)
}

pub(super) fn enforce_native_memory_limit(content: &str, max_bytes: usize) -> (String, bool) {
    if content.len() <= max_bytes {
        return (content.to_string(), false);
    }

    if max_bytes == 0 {
        return (String::new(), true);
    }

    if max_bytes <= TRUNCATION_NOTE.len() {
        return (truncate_to_byte_limit(TRUNCATION_NOTE, max_bytes), true);
    }

    let content_budget = max_bytes - TRUNCATION_NOTE.len();
    let mut capped = truncate_to_byte_limit(content, content_budget);
    capped.push_str(TRUNCATION_NOTE);
    while capped.len() > max_bytes {
        capped.pop();
    }
    (capped, true)
}

fn truncate_to_byte_limit(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }

    let mut end = 0;
    for (idx, ch) in content.char_indices() {
        let next = idx + ch.len_utf8();
        if next > max_bytes {
            break;
        }
        end = next;
    }
    content[..end].to_string()
}

pub(super) fn load_recent_sessions(
    conn: &rusqlite::Connection,
    project: &str,
) -> Result<Vec<SessionRow>> {
    let mut stmt = conn.prepare(
        "SELECT request, completed, decisions, created_at_epoch \
         FROM session_summaries \
         WHERE project = ?1 \
           AND request IS NOT NULL \
           AND request != '' \
           AND request NOT LIKE 'Captured event range %..%' \
         ORDER BY created_at_epoch DESC LIMIT ?2",
    )?;

    let rows = stmt.query_map(params![project, max_sessions() as i64], |row| {
        Ok(SessionRow {
            request: row.get(0)?,
            completed: row.get(1)?,
            decisions: row.get(2)?,
            created_at_epoch: row.get(3)?,
        })
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row?);
    }
    Ok(sessions)
}
