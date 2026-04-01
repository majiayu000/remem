use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::PathBuf;

pub fn deterministic_hash(data: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001B3;
    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub fn to_sql_refs(params: &[Box<dyn rusqlite::types::ToSql>]) -> Vec<&dyn rusqlite::types::ToSql> {
    params.iter().map(|b| b.as_ref()).collect()
}

pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

pub fn canonical_project_path(cwd: &str) -> PathBuf {
    crate::project_id::canonical_project_path(cwd)
}

pub fn project_from_cwd(cwd: &str) -> String {
    crate::project_id::project_from_cwd(cwd)
}

pub fn data_dir() -> PathBuf {
    std::env::var("REMEM_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".remem")
        })
}

pub fn db_path() -> PathBuf {
    data_dir().join("remem.db")
}

pub fn open_db() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            if let Err(e) = std::fs::set_permissions(parent, perms) {
                crate::log::warn("db", &format!("cannot set data dir permissions: {}", e));
            }
        }
    }

    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open database: {}", path.display()))?;

    if let Some(key) = super::crypto::load_cipher_key() {
        conn.pragma_update(None, "key", &key)?;
    }

    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    crate::vector::load_vec_extension(&conn)?;
    crate::migrate::run_migrations(&conn)?;
    crate::vector::ensure_vec_table(&conn)?;
    Ok(conn)
}

pub fn detect_git_branch(cwd: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() || branch == "HEAD" {
        None
    } else {
        Some(branch)
    }
}

pub fn detect_git_commit(cwd: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}
