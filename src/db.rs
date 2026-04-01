// Re-export submodules so callers can still use `db::xxx` paths.
pub use crate::db_job::*;
pub use crate::db_models::*;
pub use crate::db_pending::*;
pub use crate::db_query::*;
pub use crate::db_usage::*;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::PathBuf;

/// FNV-1a deterministic hash — stable across processes (unlike DefaultHasher).
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

/// Convert boxed params to borrowed refs for rusqlite query execution.
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

/// Build canonical project key from cwd.
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

/// Load SQLCipher encryption key from env var or key file.
/// Returns None if no encryption is configured (backward compatible).
fn load_cipher_key() -> Option<String> {
    // Priority 1: environment variable
    if let Ok(key) = std::env::var("REMEM_CIPHER_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }
    // Priority 2: key file in data directory
    let key_path = data_dir().join(".key");
    if key_path.exists() {
        if let Ok(key) = std::fs::read_to_string(&key_path) {
            let key = key.trim().to_string();
            if !key.is_empty() {
                return Some(key);
            }
        }
    }
    None
}

/// Generate a random encryption key and save to key file.
/// Returns the generated key.
pub fn generate_cipher_key() -> Result<String> {
    generate_cipher_key_with(getrandom::fill)
}

fn generate_cipher_key_with<F>(fill_random: F) -> Result<String>
where
    F: FnOnce(&mut [u8]) -> std::result::Result<(), getrandom::Error>,
{
    use std::io::Write;
    let mut key_bytes = [0u8; 32];
    fill_random(&mut key_bytes).map_err(|e| {
        anyhow::anyhow!(
            "OS randomness unavailable while generating cipher key: {}",
            e
        )
    })?;
    let key: String = key_bytes
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect();

    std::fs::create_dir_all(data_dir())?;
    let key_path = data_dir().join(".key");
    let mut f = std::fs::File::create(&key_path)?;
    f.write_all(key.as_bytes())?;
    // Restrict key file to owner only
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(&key_path, perms) {
            crate::log::warn("db", &format!("cannot set key file permissions: {}", e));
        }
    }
    Ok(key)
}

/// Encrypt an existing unencrypted database.
/// Creates an encrypted copy, then replaces the original.
pub fn encrypt_database(key: &str) -> Result<()> {
    let db_file = db_path();
    if !db_file.exists() {
        anyhow::bail!("database not found: {}", db_file.display());
    }

    let encrypted_path = db_file.with_extension("db.enc");

    // Open original DB (unencrypted)
    let conn = Connection::open(&db_file)?;
    conn.execute_batch("PRAGMA busy_timeout=5000;")?;

    // Attach encrypted copy and export
    conn.execute(
        &format!(
            "ATTACH DATABASE '{}' AS encrypted KEY '{}'",
            encrypted_path.display(),
            key.replace('\'', "''")
        ),
        [],
    )?;
    conn.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()))?;
    conn.execute("DETACH DATABASE encrypted", [])?;
    drop(conn);

    // Replace original with encrypted version
    let backup_path = db_file.with_extension("db.bak");
    std::fs::rename(&db_file, &backup_path)?;
    std::fs::rename(&encrypted_path, &db_file)?;

    crate::log::info(
        "encrypt",
        &format!("database encrypted, backup at {}", backup_path.display()),
    );
    Ok(())
}

pub fn open_db() -> Result<Connection> {
    let path = db_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        // Restrict data directory to owner only (rwx------)
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

    // Apply SQLCipher encryption key if configured
    if let Some(key) = load_cipher_key() {
        conn.pragma_update(None, "key", &key)?;
    }

    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;

    // Load sqlite-vec extension
    crate::vector::load_vec_extension(&conn)?;

    // Run numbered migrations (creates tables, indexes, etc.)
    crate::migrate::run_migrations(&conn)?;

    // Vector table depends on sqlite-vec extension, handled separately
    crate::vector::ensure_vec_table(&conn)?;

    Ok(conn)
}

/// Detect the current git branch from a working directory.
/// Returns None if not in a git repo or git is not available.
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
        // Detached HEAD — not a named branch
        None
    } else {
        Some(branch)
    }
}

/// Detect the current short commit SHA from a working directory.
/// Returns None if not in a git repo or git is not available.
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

// --- Summarize rate limiting ---

/// 检查项目是否在冷却期内。返回 true = 应该跳过。
pub fn is_summarize_on_cooldown(
    conn: &Connection,
    project: &str,
    cooldown_secs: i64,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let result: rusqlite::Result<i64> = conn.query_row(
        "SELECT last_summarize_epoch FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    );

    match result {
        Ok(last_epoch) => Ok(now - last_epoch < cooldown_secs),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
pub mod test_support {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    pub struct ScopedTestDataDir {
        _guard: MutexGuard<'static, ()>,
        previous: Option<OsString>,
        pub path: PathBuf,
    }

    impl ScopedTestDataDir {
        pub fn new(label: &str) -> Self {
            let guard = env_lock().lock().expect("test env lock poisoned");
            let previous = std::env::var_os("REMEM_DATA_DIR");
            let unique = format!(
                "remem-test-{}-{}-{}",
                label,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system time before unix epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&path);
            std::env::set_var("REMEM_DATA_DIR", &path);
            Self {
                _guard: guard,
                previous,
                path,
            }
        }

        pub fn db_path(&self) -> PathBuf {
            self.path.join("remem.db")
        }

        pub fn remove_db_files(&self) {
            let db_path = self.db_path();
            let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
            let shm_path = PathBuf::from(format!("{}-shm", db_path.display()));
            for path in [db_path, wal_path, shm_path] {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    impl Drop for ScopedTestDataDir {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("REMEM_DATA_DIR", previous);
            } else {
                std::env::remove_var("REMEM_DATA_DIR");
            }
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// 检查 message hash 是否与上次相同。返回 true = 重复消息，应该跳过。
pub fn is_duplicate_message(conn: &Connection, project: &str, message_hash: &str) -> Result<bool> {
    let result: rusqlite::Result<Option<String>> = conn.query_row(
        "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
        params![project],
        |row| row.get(0),
    );

    match result {
        Ok(Some(prev_hash)) => Ok(prev_hash == message_hash),
        Ok(None) => Ok(false),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Try to acquire a short-lived summarize lock for one project.
/// Returns false when another worker currently owns a non-expired lock.
pub fn try_acquire_summarize_lock(
    conn: &mut Connection,
    project: &str,
    lock_secs: i64,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let lock_secs = lock_secs.max(1);
    let tx = conn.transaction()?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT lock_epoch FROM summarize_locks WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(epoch) = existing {
        if now - epoch < lock_secs {
            tx.rollback()?;
            return Ok(false);
        }
    }
    tx.execute(
        "INSERT INTO summarize_locks (project, lock_epoch)
         VALUES (?1, ?2)
         ON CONFLICT(project) DO UPDATE SET lock_epoch = ?2",
        params![project, now],
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn release_summarize_lock(conn: &Connection, project: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM summarize_locks WHERE project = ?1",
        params![project],
    )?;
    Ok(())
}

/// 原子替换 summary + 更新 summarize 冷却/去重 gate。
/// 返回值为被替换掉的旧 summary 条数。
pub fn finalize_summarize(
    conn: &mut Connection,
    memory_session_id: &str,
    project: &str,
    message_hash: &str,
    request: Option<&str>,
    completed: Option<&str>,
    decisions: Option<&str>,
    learned: Option<&str>,
    next_steps: Option<&str>,
    preferences: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
) -> Result<usize> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();

    let tx = conn.transaction()?;
    let deleted = tx.execute(
        "DELETE FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
        params![memory_session_id, project],
    )?;
    tx.execute(
        "INSERT INTO session_summaries \
         (memory_session_id, project, request, completed, decisions, learned, \
          next_steps, preferences, prompt_number, created_at, created_at_epoch, discovery_tokens) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            memory_session_id,
            project,
            request,
            completed,
            decisions,
            learned,
            next_steps,
            preferences,
            prompt_number,
            created_at,
            created_at_epoch,
            discovery_tokens
        ],
    )?;
    tx.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?2,
           last_message_hash = ?3",
        params![project, created_at_epoch, message_hash],
    )?;
    tx.commit()?;
    Ok(deleted)
}

pub fn insert_observation(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
    obs_type: &str,
    title: Option<&str>,
    subtitle: Option<&str>,
    narrative: Option<&str>,
    facts: Option<&str>,
    concepts: Option<&str>,
    files_read: Option<&str>,
    files_modified: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
) -> Result<i64> {
    insert_observation_with_branch(
        conn,
        memory_session_id,
        project,
        obs_type,
        title,
        subtitle,
        narrative,
        facts,
        concepts,
        files_read,
        files_modified,
        prompt_number,
        discovery_tokens,
        None,
        None,
    )
}

pub fn insert_observation_with_branch(
    conn: &Connection,
    memory_session_id: &str,
    project: &str,
    obs_type: &str,
    title: Option<&str>,
    subtitle: Option<&str>,
    narrative: Option<&str>,
    facts: Option<&str>,
    concepts: Option<&str>,
    files_read: Option<&str>,
    files_modified: Option<&str>,
    prompt_number: Option<i64>,
    discovery_tokens: i64,
    branch: Option<&str>,
    commit_sha: Option<&str>,
) -> Result<i64> {
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let created_at_epoch = now.timestamp();

    conn.execute(
        "INSERT INTO observations \
         (memory_session_id, project, type, title, subtitle, narrative, \
          facts, concepts, files_read, files_modified, prompt_number, \
          created_at, created_at_epoch, discovery_tokens, branch, commit_sha) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            memory_session_id,
            project,
            obs_type,
            title,
            subtitle,
            narrative,
            facts,
            concepts,
            files_read,
            files_modified,
            prompt_number,
            created_at,
            created_at_epoch,
            discovery_tokens,
            branch,
            commit_sha
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn mark_stale_by_files(
    conn: &Connection,
    new_obs_id: i64,
    project: &str,
    files_modified: &[String],
) -> Result<usize> {
    if files_modified.is_empty() {
        return Ok(0);
    }
    let files_json = serde_json::to_string(files_modified)?;
    let count = conn.execute(
        "UPDATE observations SET status = 'stale'
         WHERE id != ?1 AND project = ?2 AND status = 'active'
           AND id IN (
             SELECT DISTINCT o.id FROM observations o, json_each(o.files_modified) AS old_f
             WHERE o.id != ?1 AND o.project = ?2 AND o.status = 'active'
               AND o.files_modified IS NOT NULL AND length(o.files_modified) > 2
               AND old_f.value IN (SELECT value FROM json_each(?3))
           )",
        params![new_obs_id, project, files_json],
    )?;
    Ok(count)
}

/// Mark observations as compressed (they won't appear in context loading).
pub fn mark_observations_compressed(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE observations SET status = 'compressed' WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    let refs = to_sql_refs(&param_values);
    let count = stmt.execute(refs.as_slice())?;
    Ok(count)
}

pub fn update_last_accessed(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    let placeholders: Vec<String> = (2..=ids.len() + 1).map(|i| format!("?{i}")).collect();
    let sql = format!(
        "UPDATE observations SET last_accessed_epoch = ?1 WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(now));
    for id in ids {
        param_values.push(Box::new(*id));
    }
    let refs = to_sql_refs(&param_values);
    stmt.execute(refs.as_slice())?;
    Ok(())
}

pub fn upsert_session(
    conn: &Connection,
    content_session_id: &str,
    project: &str,
    user_prompt: Option<&str>,
) -> Result<String> {
    let now = chrono::Utc::now();
    let started_at = now.to_rfc3339();
    let started_at_epoch = now.timestamp();
    let memory_session_id = format!("mem-{}", truncate_str(content_session_id, 8));

    conn.execute(
        "INSERT INTO sdk_sessions \
         (content_session_id, memory_session_id, project, user_prompt, \
          started_at, started_at_epoch, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active') \
         ON CONFLICT(content_session_id) DO UPDATE SET \
         prompt_counter = prompt_counter + 1",
        params![
            content_session_id,
            memory_session_id,
            project,
            user_prompt,
            started_at,
            started_at_epoch
        ],
    )?;

    let mid: String = conn.query_row(
        "SELECT memory_session_id FROM sdk_sessions WHERE content_session_id = ?1",
        params![content_session_id],
        |row| row.get(0),
    )?;
    Ok(mid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn setup_summary_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE session_summaries (
                id INTEGER PRIMARY KEY,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                request TEXT,
                completed TEXT,
                decisions TEXT,
                learned TEXT,
                next_steps TEXT,
                preferences TEXT,
                prompt_number INTEGER,
                created_at TEXT,
                created_at_epoch INTEGER,
                discovery_tokens INTEGER DEFAULT 0
            );
            CREATE TABLE summarize_cooldown (
                project TEXT PRIMARY KEY,
                last_summarize_epoch INTEGER NOT NULL,
                last_message_hash TEXT
            );",
        )?;
        Ok(())
    }

    #[test]
    fn finalize_summarize_replaces_in_single_commit() -> Result<()> {
        let mut conn = Connection::open_in_memory()?;
        setup_summary_schema(&conn)?;
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch, discovery_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["mem-1", "proj", "old", "2026-01-01T00:00:00Z", 1_i64, 10_i64],
        )?;

        let deleted = finalize_summarize(
            &mut conn,
            "mem-1",
            "proj",
            "hash-1",
            Some("new"),
            Some("done"),
            Some("decision"),
            Some("learned"),
            Some("next"),
            Some("pref"),
            None,
            99,
        )?;
        assert_eq!(deleted, 1);

        let req: String = conn.query_row(
            "SELECT request FROM session_summaries WHERE memory_session_id = ?1 AND project = ?2",
            params!["mem-1", "proj"],
            |r| r.get(0),
        )?;
        assert_eq!(req, "new");

        let hash: String = conn.query_row(
            "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
            params!["proj"],
            |r| r.get(0),
        )?;
        assert_eq!(hash, "hash-1");
        Ok(())
    }

    #[test]
    fn generate_cipher_key_writes_64_hex_chars() -> Result<()> {
        let test_dir = test_support::ScopedTestDataDir::new("cipher-key");
        std::fs::create_dir_all(&test_dir.path)?;

        let key = generate_cipher_key()?;
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|ch| ch.is_ascii_hexdigit()));

        let saved = std::fs::read_to_string(test_dir.path.join(".key"))?;
        assert_eq!(saved, key);
        Ok(())
    }

    #[test]
    fn generate_cipher_key_fails_when_os_randomness_is_unavailable() {
        let test_dir = test_support::ScopedTestDataDir::new("cipher-key-fail");
        std::fs::create_dir_all(&test_dir.path).expect("test data dir should exist");

        let err = generate_cipher_key_with(|_| Err(getrandom::Error::UNSUPPORTED))
            .expect_err("cipher key generation should fail without OS randomness");

        assert!(err.to_string().contains("OS randomness unavailable"));
        assert!(!test_dir.path.join(".key").exists());
    }
}
