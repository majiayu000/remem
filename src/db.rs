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

/// Current schema version — bump when adding migrations.
const SCHEMA_VERSION: i64 = 13;

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
    use std::io::Write;
    let key: String = (0..32).map(|_| format!("{:02x}", rand_byte())).collect();
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

/// Simple random byte from /dev/urandom or fallback to time-based.
fn rand_byte() -> u8 {
    use std::io::Read;
    let mut buf = [0u8; 1];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut buf).is_ok() {
            return buf[0];
        }
    }
    // Fallback: time-based (not cryptographically secure, but functional)
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos()
        & 0xFF) as u8
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
    conn.execute(&format!("DETACH DATABASE encrypted"), [])?;
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

    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < SCHEMA_VERSION {
        ensure_core_schema(&conn)?;
        ensure_pending_table(&conn)?;
        crate::vector::ensure_vec_table(&conn)?;
        ensure_schema_migrations(&conn, version)?;
        conn.execute_batch(&format!("PRAGMA user_version = {}", SCHEMA_VERSION))?;
    }

    Ok(conn)
}

fn ensure_core_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sdk_sessions (
            id INTEGER PRIMARY KEY,
            content_session_id TEXT UNIQUE NOT NULL,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            user_prompt TEXT,
            started_at TEXT,
            started_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            prompt_counter INTEGER DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS observations (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT,
            type TEXT NOT NULL,
            title TEXT,
            subtitle TEXT,
            narrative TEXT,
            facts TEXT,
            concepts TEXT,
            files_read TEXT,
            files_modified TEXT,
            prompt_number INTEGER,
            created_at TEXT,
            created_at_epoch INTEGER,
            discovery_tokens INTEGER DEFAULT 0,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER
        );

        CREATE TABLE IF NOT EXISTS session_summaries (
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

        CREATE VIRTUAL TABLE IF NOT EXISTS observations_fts USING fts5(
            title, subtitle, narrative, facts, concepts,
            content='observations',
            content_rowid='id',
            tokenize='trigram'
        );

        CREATE TRIGGER IF NOT EXISTS observations_ai AFTER INSERT ON observations BEGIN
            INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
            VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
        END;

        CREATE TRIGGER IF NOT EXISTS observations_ad AFTER DELETE ON observations BEGIN
            INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
            VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
        END;

        CREATE TRIGGER IF NOT EXISTS observations_au AFTER UPDATE ON observations BEGIN
            INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
            VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
            INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
            VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
        END;"
    )?;
    Ok(())
}

fn ensure_pending_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pending_observations (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            tool_input TEXT,
            tool_response TEXT,
            cwd TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
            status TEXT NOT NULL DEFAULT 'pending',
            attempt_count INTEGER NOT NULL DEFAULT 0,
            next_retry_epoch INTEGER,
            last_error TEXT,
            lease_owner TEXT,
            lease_expires_epoch INTEGER
        )",
    )?;
    Ok(())
}

fn ensure_schema_migrations(conn: &Connection, old_version: i64) -> Result<()> {
    let migrations: &[(&str, &str, &str)] = &[
        (
            "observations",
            "status",
            "ALTER TABLE observations ADD COLUMN status TEXT DEFAULT 'active'",
        ),
        (
            "observations",
            "last_accessed_epoch",
            "ALTER TABLE observations ADD COLUMN last_accessed_epoch INTEGER",
        ),
        (
            "session_summaries",
            "decisions",
            "ALTER TABLE session_summaries ADD COLUMN decisions TEXT",
        ),
        (
            "session_summaries",
            "preferences",
            "ALTER TABLE session_summaries ADD COLUMN preferences TEXT",
        ),
        (
            "pending_observations",
            "lease_owner",
            "ALTER TABLE pending_observations ADD COLUMN lease_owner TEXT",
        ),
        (
            "pending_observations",
            "lease_expires_epoch",
            "ALTER TABLE pending_observations ADD COLUMN lease_expires_epoch INTEGER",
        ),
        (
            "pending_observations",
            "updated_at_epoch",
            "ALTER TABLE pending_observations ADD COLUMN updated_at_epoch INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "pending_observations",
            "status",
            "ALTER TABLE pending_observations ADD COLUMN status TEXT NOT NULL DEFAULT 'pending'",
        ),
        (
            "pending_observations",
            "attempt_count",
            "ALTER TABLE pending_observations ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "pending_observations",
            "next_retry_epoch",
            "ALTER TABLE pending_observations ADD COLUMN next_retry_epoch INTEGER",
        ),
        (
            "pending_observations",
            "last_error",
            "ALTER TABLE pending_observations ADD COLUMN last_error TEXT",
        ),
    ];
    for (table, col, sql) in migrations {
        if !column_exists(conn, table, col)? {
            conn.execute_batch(sql)?;
        }
    }

    // v9: Remove @hash suffix from project fields
    if old_version < 9 {
        crate::log::info("db", "migrating to v9: removing @hash from project fields");
        conn.execute_batch(
            "UPDATE observations SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';
             UPDATE memories SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';
             UPDATE events SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';
             UPDATE session_summaries SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';
             UPDATE sdk_sessions SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';
             UPDATE workstreams SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';
             UPDATE pending_observations SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';
             UPDATE jobs SET project = substr(project, 1, instr(project || '@', '@') - 1) WHERE project LIKE '%@%';"
        )?;
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_observations_status ON observations(status);
         CREATE INDEX IF NOT EXISTS idx_observations_project_status
           ON observations(project, status, created_at_epoch DESC);
         CREATE INDEX IF NOT EXISTS idx_pending_session_lease
           ON pending_observations(session_id, lease_expires_epoch, id);
         CREATE INDEX IF NOT EXISTS idx_pending_project_lease
           ON pending_observations(project, lease_expires_epoch, created_at_epoch);
         CREATE INDEX IF NOT EXISTS idx_pending_claim_v2
           ON pending_observations(status, session_id, next_retry_epoch, lease_expires_epoch, id);
         CREATE INDEX IF NOT EXISTS idx_pending_project_v2
           ON pending_observations(status, project, next_retry_epoch, created_at_epoch);
         CREATE INDEX IF NOT EXISTS idx_sdk_sessions_msid
           ON sdk_sessions(memory_session_id);

         CREATE TABLE IF NOT EXISTS summarize_cooldown (
             project TEXT PRIMARY KEY,
             last_summarize_epoch INTEGER NOT NULL,
             last_message_hash TEXT
         );

         CREATE TABLE IF NOT EXISTS summarize_locks (
             project TEXT PRIMARY KEY,
             lock_epoch INTEGER NOT NULL
         );

         CREATE TABLE IF NOT EXISTS ai_usage_events (
             id INTEGER PRIMARY KEY,
             created_at TEXT NOT NULL,
             created_at_epoch INTEGER NOT NULL,
             project TEXT,
             operation TEXT NOT NULL,
             executor TEXT NOT NULL,
             model TEXT,
             input_tokens INTEGER NOT NULL,
             output_tokens INTEGER NOT NULL,
             total_tokens INTEGER NOT NULL,
             estimated_cost_usd REAL NOT NULL
         );

         CREATE INDEX IF NOT EXISTS idx_ai_usage_created
           ON ai_usage_events(created_at_epoch DESC);
         CREATE INDEX IF NOT EXISTS idx_ai_usage_project_created
           ON ai_usage_events(project, created_at_epoch DESC);

         CREATE TABLE IF NOT EXISTS jobs (
             id INTEGER PRIMARY KEY,
             job_type TEXT NOT NULL,
             project TEXT NOT NULL,
             session_id TEXT,
             payload_json TEXT NOT NULL,
             state TEXT NOT NULL,
             priority INTEGER NOT NULL DEFAULT 100,
             attempt_count INTEGER NOT NULL DEFAULT 0,
             max_attempts INTEGER NOT NULL DEFAULT 6,
             lease_owner TEXT,
             lease_expires_epoch INTEGER,
             next_retry_epoch INTEGER NOT NULL DEFAULT 0,
             last_error TEXT,
             created_at_epoch INTEGER NOT NULL,
             updated_at_epoch INTEGER NOT NULL
         );

         CREATE INDEX IF NOT EXISTS idx_jobs_claim
           ON jobs(state, next_retry_epoch, priority, created_at_epoch, id);
         CREATE INDEX IF NOT EXISTS idx_jobs_project_state
           ON jobs(project, state, created_at_epoch DESC);
         CREATE INDEX IF NOT EXISTS idx_jobs_lease
           ON jobs(state, lease_expires_epoch);",
    )?;
    migrate_session_summaries_v4(conn)?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workstreams (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            title TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL DEFAULT 'active',
            progress TEXT,
            next_action TEXT,
            blockers TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            completed_at_epoch INTEGER
        );
        CREATE TABLE IF NOT EXISTS workstream_sessions (
            id INTEGER PRIMARY KEY,
            workstream_id INTEGER NOT NULL,
            memory_session_id TEXT NOT NULL,
            linked_at_epoch INTEGER NOT NULL,
            UNIQUE(workstream_id, memory_session_id)
        );
        CREATE INDEX IF NOT EXISTS idx_workstreams_project_status
          ON workstreams(project, status, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_workstream_sessions_ws
          ON workstream_sessions(workstream_id);
        CREATE INDEX IF NOT EXISTS idx_workstream_sessions_session
          ON workstream_sessions(memory_session_id);",
    )?;

    if old_version > 0 && old_version < 7 {
        migrate_fts_trigram(conn)?;
    }

    if old_version < 8 {
        migrate_to_v8(conn)?;
    }

    if old_version < 10 {
        migrate_to_v10(conn)?;
    }

    // v11: Add scope column to memories (project vs global)
    if !column_exists(conn, "memories", "scope")? {
        conn.execute_batch(
            "ALTER TABLE memories ADD COLUMN scope TEXT DEFAULT 'project';
             CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope, status, updated_at_epoch DESC);",
        )?;
    }

    // v12: Entity index for entity-aware retrieval
    if old_version < 12 {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS entities (
                id INTEGER PRIMARY KEY,
                canonical_name TEXT NOT NULL COLLATE NOCASE,
                entity_type TEXT,
                mention_count INTEGER DEFAULT 1,
                created_at_epoch INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
                UNIQUE(canonical_name)
            );
            CREATE TABLE IF NOT EXISTS memory_entities (
                memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                entity_id INTEGER NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
                PRIMARY KEY(memory_id, entity_id)
            );
            CREATE INDEX IF NOT EXISTS idx_entity_name ON entities(canonical_name COLLATE NOCASE);
            CREATE INDEX IF NOT EXISTS idx_memory_entities_entity ON memory_entities(entity_id);",
        )?;
    }

    // v13: Recoverable pending queue state machine.
    if old_version < 13 {
        conn.execute_batch(
            "UPDATE pending_observations
             SET status = 'pending'
             WHERE status IS NULL OR status = '';
             UPDATE pending_observations
             SET attempt_count = COALESCE(attempt_count, 0),
                 updated_at_epoch = COALESCE(updated_at_epoch, created_at_epoch),
                 next_retry_epoch = COALESCE(next_retry_epoch, created_at_epoch)
             WHERE 1=1;",
        )?;
    }

    Ok(())
}

/// Schema v8: Add memories + events tables with FTS for the new memory quality system.
fn migrate_to_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
            id INTEGER PRIMARY KEY,
            session_id TEXT,
            project TEXT NOT NULL,
            topic_key TEXT,
            title TEXT NOT NULL,
            content TEXT NOT NULL,
            memory_type TEXT NOT NULL,
            files TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active'
        );

        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            event_type TEXT NOT NULL,
            summary TEXT NOT NULL,
            detail TEXT,
            files TEXT,
            exit_code INTEGER,
            created_at_epoch INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_memories_project_status
          ON memories(project, status, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_memories_topic_key
          ON memories(project, topic_key) WHERE topic_key IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_memories_type
          ON memories(project, memory_type, updated_at_epoch DESC);
        CREATE INDEX IF NOT EXISTS idx_events_session
          ON events(session_id, created_at_epoch);
        CREATE INDEX IF NOT EXISTS idx_events_project
          ON events(project, created_at_epoch DESC);

        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
            title, content,
            content='memories',
            content_rowid='id',
            tokenize='trigram'
        );

        CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
            INSERT INTO memories_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;",
    )?;

    // Migrate manual observations (from save_memory MCP calls) to memories table.
    let manual_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM observations WHERE memory_session_id = 'manual' AND status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if manual_count > 0 {
        conn.execute_batch(
            "INSERT INTO memories (session_id, project, title, content, memory_type, \
             created_at_epoch, updated_at_epoch, status)
             SELECT memory_session_id, COALESCE(project, 'manual'), \
                    COALESCE(title, 'Memory'), COALESCE(narrative, ''), \
                    COALESCE(type, 'discovery'), \
                    created_at_epoch, created_at_epoch, 'active'
             FROM observations
             WHERE memory_session_id = 'manual' AND status = 'active'",
        )?;
        crate::log::info(
            "migrate",
            &format!("migrated {} manual observations to memories", manual_count),
        );
    }

    Ok(())
}

/// Schema v10: Add branch/commit_sha columns to observations and memories for git branch isolation.
fn migrate_to_v10(conn: &Connection) -> Result<()> {
    let migrations: &[(&str, &str, &str)] = &[
        (
            "observations",
            "branch",
            "ALTER TABLE observations ADD COLUMN branch TEXT",
        ),
        (
            "observations",
            "commit_sha",
            "ALTER TABLE observations ADD COLUMN commit_sha TEXT",
        ),
        (
            "memories",
            "branch",
            "ALTER TABLE memories ADD COLUMN branch TEXT",
        ),
    ];
    for (table, col, sql) in migrations {
        if !column_exists(conn, table, col)? {
            conn.execute_batch(sql)?;
        }
    }

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_observations_branch
           ON observations(project, branch, created_at_epoch DESC);
         CREATE INDEX IF NOT EXISTS idx_memories_branch
           ON memories(project, branch, updated_at_epoch DESC);",
    )?;

    crate::log::info("db", "migrated to v10: added branch/commit_sha columns");
    Ok(())
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

/// Migrate FTS table from unicode61 to trigram tokenizer for CJK support.
fn migrate_fts_trigram(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS observations_ai;
         DROP TRIGGER IF EXISTS observations_ad;
         DROP TRIGGER IF EXISTS observations_au;
         DROP TABLE IF EXISTS observations_fts;

         CREATE VIRTUAL TABLE observations_fts USING fts5(
             title, subtitle, narrative, facts, concepts,
             content='observations',
             content_rowid='id',
             tokenize='trigram'
         );

         CREATE TRIGGER observations_ai AFTER INSERT ON observations BEGIN
             INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
             VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
         END;

         CREATE TRIGGER observations_ad AFTER DELETE ON observations BEGIN
             INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
             VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
         END;

         CREATE TRIGGER observations_au AFTER UPDATE ON observations BEGIN
             INSERT INTO observations_fts(observations_fts, rowid, title, subtitle, narrative, facts, concepts)
             VALUES ('delete', old.id, old.title, old.subtitle, old.narrative, old.facts, old.concepts);
             INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
             VALUES (new.id, new.title, new.subtitle, new.narrative, new.facts, new.concepts);
         END;

         INSERT INTO observations_fts(rowid, title, subtitle, narrative, facts, concepts)
         SELECT id, title, subtitle, narrative, facts, concepts FROM observations;",
    )?;
    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn migrate_session_summaries_v4(conn: &Connection) -> Result<()> {
    let has_investigated = column_exists(conn, "session_summaries", "investigated")?;
    let has_notes = column_exists(conn, "session_summaries", "notes")?;
    if !has_investigated && !has_notes {
        return Ok(());
    }

    let completed_expr = if has_investigated {
        "COALESCE(completed, investigated)"
    } else {
        "completed"
    };
    let preferences_expr = if has_notes {
        "COALESCE(preferences, notes)"
    } else {
        "preferences"
    };

    let sql = format!(
        "BEGIN IMMEDIATE;
         DROP TABLE IF EXISTS session_summaries_v4;
         CREATE TABLE session_summaries_v4 (
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
         INSERT INTO session_summaries_v4
             (id, memory_session_id, project, request, completed, decisions, learned,
              next_steps, preferences, prompt_number, created_at, created_at_epoch, discovery_tokens)
         SELECT id, memory_session_id, project, request, {completed_expr}, decisions, learned,
                next_steps, {preferences_expr}, prompt_number, created_at, created_at_epoch, discovery_tokens
         FROM session_summaries;
         DROP TABLE session_summaries;
         ALTER TABLE session_summaries_v4 RENAME TO session_summaries;
         CREATE INDEX IF NOT EXISTS idx_session_summaries_project_created
           ON session_summaries(project, created_at_epoch DESC);
         COMMIT;"
    );
    conn.execute_batch(&sql)?;
    Ok(())
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

    fn setup_legacy_summary_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE session_summaries (
                id INTEGER PRIMARY KEY,
                memory_session_id TEXT NOT NULL,
                project TEXT,
                request TEXT,
                investigated TEXT,
                learned TEXT,
                completed TEXT,
                next_steps TEXT,
                notes TEXT,
                prompt_number INTEGER,
                created_at TEXT,
                created_at_epoch INTEGER,
                discovery_tokens INTEGER DEFAULT 0,
                decisions TEXT,
                preferences TEXT
            );",
        )?;
        Ok(())
    }

    #[test]
    fn migrate_legacy_summary_columns_to_v4_shape() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_legacy_summary_schema(&conn)?;
        conn.execute(
            "INSERT INTO session_summaries
             (memory_session_id, project, request, investigated, learned, next_steps, notes,
              created_at, created_at_epoch, discovery_tokens, decisions, preferences)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)",
            params![
                "mem-legacy",
                "proj",
                "req",
                "legacy-completed",
                "learned",
                "next",
                "legacy-pref",
                "2026-01-01T00:00:00Z",
                1_i64,
                5_i64,
                "decision"
            ],
        )?;

        migrate_session_summaries_v4(&conn)?;

        assert!(!column_exists(&conn, "session_summaries", "investigated")?);
        assert!(!column_exists(&conn, "session_summaries", "notes")?);

        let completed: String = conn.query_row(
            "SELECT completed FROM session_summaries WHERE memory_session_id = 'mem-legacy'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(completed, "legacy-completed");

        let preferences: String = conn.query_row(
            "SELECT preferences FROM session_summaries WHERE memory_session_id = 'mem-legacy'",
            [],
            |r| r.get(0),
        )?;
        assert_eq!(preferences, "legacy-pref");
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
}
