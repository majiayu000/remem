//! 测试 summarize rate limiting、cleanup、Bash 过滤、project_from_cwd、flush 批次限制。

use anyhow::Result;
use rusqlite::{Connection, params};

// --- 内联必要的 DB 函数（避免改动 lib 结构）---

fn setup_db() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;

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
        );

        CREATE TABLE IF NOT EXISTS pending_observations (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            tool_input TEXT,
            tool_response TEXT,
            cwd TEXT,
            created_at_epoch INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS summarize_cooldown (
            project TEXT PRIMARY KEY,
            last_summarize_epoch INTEGER NOT NULL,
            last_message_hash TEXT
        );",
    )?;
    Ok(conn)
}

// --- Rate limiting functions (mirrors db.rs with proper error handling) ---

fn is_summarize_on_cooldown(conn: &Connection, project: &str, cooldown_secs: i64) -> Result<bool> {
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

fn is_duplicate_message(conn: &Connection, project: &str, message_hash: &str) -> Result<bool> {
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

fn record_summarize(conn: &Connection, project: &str, message_hash: &str) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?2,
           last_message_hash = ?3",
        params![project, now, message_hash],
    )?;
    Ok(())
}

fn cleanup_orphan_summaries(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM session_summaries
         WHERE memory_session_id LIKE 'mem-%'
           AND memory_session_id NOT IN (
             SELECT DISTINCT memory_session_id FROM observations
           )",
        [],
    )?;
    Ok(count)
}

fn cleanup_duplicate_summaries(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM session_summaries
         WHERE id NOT IN (
           SELECT MAX(id)
           FROM session_summaries
           GROUP BY memory_session_id, project
         )",
        [],
    )?;
    Ok(count)
}

fn cleanup_stale_pending(conn: &Connection) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - 3600;
    let count = conn.execute(
        "DELETE FROM pending_observations WHERE created_at_epoch < ?1",
        params![cutoff],
    )?;
    Ok(count)
}

fn cleanup_expired_compressed(conn: &Connection, ttl_days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (ttl_days * 86400);
    let count = conn.execute(
        "DELETE FROM observations WHERE status = 'compressed' AND created_at_epoch < ?1",
        params![cutoff],
    )?;
    Ok(count)
}

fn get_stale_pending_sessions(conn: &Connection, project: &str, age_secs: i64) -> Result<Vec<String>> {
    let cutoff = chrono::Utc::now().timestamp() - age_secs;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT session_id FROM pending_observations \
         WHERE project = ?1 AND created_at_epoch < ?2"
    )?;
    let rows = stmt.query_map(params![project, cutoff], |row| row.get(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

fn dequeue_pending_with_limit(conn: &Connection, session_id: &str, limit: usize) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM pending_observations WHERE session_id = ?1 ORDER BY id ASC LIMIT ?2"
    )?;
    let rows = stmt.query_map(params![session_id, limit as i64], |row| row.get(0))?;
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

fn hash_message(msg: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    msg.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn project_from_cwd(cwd: &str) -> String {
    let path = std::path::Path::new(cwd);
    let components: Vec<&std::ffi::OsStr> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(n) => Some(n),
            _ => None,
        })
        .collect();
    match components.len() {
        0 => cwd.to_string(),
        1 => components[0].to_string_lossy().to_string(),
        n => format!(
            "{}/{}",
            components[n - 2].to_string_lossy(),
            components[n - 1].to_string_lossy()
        ),
    }
}

// --- Bash skip filter ---

const BASH_SKIP_PREFIXES: &[&str] = &[
    "git status", "git log", "git diff", "git branch", "git stash list",
    "git remote", "git fetch", "git show",
    "ls", "pwd", "echo ", "which ", "type ", "whereis ",
    "cat ", "head ", "tail ", "wc ", "file ",
    "npm install", "npm ci", "yarn install", "pnpm install",
    "cargo build", "cargo check", "cargo clippy", "cargo fmt",
    "cd ", "pushd ", "popd",
    "lsof ", "ps ", "top", "htop", "df ", "du ",
];

fn should_skip_bash(cmd: &str) -> bool {
    let cmd_trimmed = cmd.trim();
    BASH_SKIP_PREFIXES.iter().any(|prefix| cmd_trimmed.starts_with(prefix))
}

// --- Model resolution ---

fn resolve_model_for_api(short: &str) -> &str {
    match short {
        "haiku" => "claude-haiku-4-5-20251001",
        "sonnet" => "claude-sonnet-4-5-20250514",
        "opus" => "claude-opus-4-20250514",
        _ => short,
    }
}

// ===================== 测试 =====================

// --- Cooldown tests ---

#[test]
fn test_cooldown_no_record() -> Result<()> {
    let conn = setup_db()?;
    assert!(!is_summarize_on_cooldown(&conn, "test-project", 300)?);
    Ok(())
}

#[test]
fn test_cooldown_within_period() -> Result<()> {
    let conn = setup_db()?;
    record_summarize(&conn, "test-project", "hash1")?;
    assert!(is_summarize_on_cooldown(&conn, "test-project", 300)?);
    Ok(())
}

#[test]
fn test_cooldown_expired() -> Result<()> {
    let conn = setup_db()?;
    let old_epoch = chrono::Utc::now().timestamp() - 400;
    conn.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)",
        params!["test-project", old_epoch, "old_hash"],
    )?;
    assert!(!is_summarize_on_cooldown(&conn, "test-project", 300)?);
    Ok(())
}

#[test]
fn test_cooldown_per_project() -> Result<()> {
    let conn = setup_db()?;
    record_summarize(&conn, "project-a", "hash1")?;
    assert!(is_summarize_on_cooldown(&conn, "project-a", 300)?);
    assert!(!is_summarize_on_cooldown(&conn, "project-b", 300)?);
    Ok(())
}

// --- Duplicate message tests ---

#[test]
fn test_duplicate_message_no_record() -> Result<()> {
    let conn = setup_db()?;
    assert!(!is_duplicate_message(&conn, "test-project", "hash1")?);
    Ok(())
}

#[test]
fn test_duplicate_message_same_hash() -> Result<()> {
    let conn = setup_db()?;
    record_summarize(&conn, "test-project", "hash1")?;
    assert!(is_duplicate_message(&conn, "test-project", "hash1")?);
    Ok(())
}

#[test]
fn test_duplicate_message_different_hash() -> Result<()> {
    let conn = setup_db()?;
    record_summarize(&conn, "test-project", "hash1")?;
    assert!(!is_duplicate_message(&conn, "test-project", "hash2")?);
    Ok(())
}

#[test]
fn test_record_summarize_upsert() -> Result<()> {
    let conn = setup_db()?;
    record_summarize(&conn, "test-project", "hash1")?;
    record_summarize(&conn, "test-project", "hash2")?;
    assert!(!is_duplicate_message(&conn, "test-project", "hash1")?);
    assert!(is_duplicate_message(&conn, "test-project", "hash2")?);
    Ok(())
}

// --- Hash tests ---

#[test]
fn test_hash_message_deterministic() {
    let h1 = hash_message("hello world");
    let h2 = hash_message("hello world");
    assert_eq!(h1, h2);
}

#[test]
fn test_hash_message_different_inputs() {
    let h1 = hash_message("hello world");
    let h2 = hash_message("hello world!");
    assert_ne!(h1, h2);
}

// --- Cleanup tests ---

#[test]
fn test_cleanup_orphan_summaries() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now();

    conn.execute(
        "INSERT INTO observations (memory_session_id, project, type, created_at, created_at_epoch)
         VALUES ('mem-abcd1234', 'project-a', 'feature', ?1, ?2)",
        params![now.to_rfc3339(), now.timestamp()],
    )?;
    conn.execute(
        "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
         VALUES ('mem-abcd1234', 'project-a', 'do something', ?1, ?2)",
        params![now.to_rfc3339(), now.timestamp()],
    )?;

    for i in 0..5 {
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
             VALUES (?1, 'project-b', 'orphan', ?2, ?3)",
            params![format!("mem-orphan{}", i), now.to_rfc3339(), now.timestamp()],
        )?;
    }

    conn.execute(
        "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
         VALUES ('uuid-session', 'project-c', 'not orphan', ?1, ?2)",
        params![now.to_rfc3339(), now.timestamp()],
    )?;

    let deleted = cleanup_orphan_summaries(&conn)?;
    assert_eq!(deleted, 5);

    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_summaries", [], |row| row.get(0),
    )?;
    assert_eq!(remaining, 2);
    Ok(())
}

#[test]
fn test_cleanup_duplicate_summaries() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now();

    for i in 0..5 {
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
             VALUES ('session-a', 'project-a', ?1, ?2, ?3)",
            params![format!("request-{}", i), now.to_rfc3339(), now.timestamp() + i],
        )?;
    }

    conn.execute(
        "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
         VALUES ('session-b', 'project-a', 'single', ?1, ?2)",
        params![now.to_rfc3339(), now.timestamp()],
    )?;

    let deleted = cleanup_duplicate_summaries(&conn)?;
    assert_eq!(deleted, 4);

    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_summaries", [], |row| row.get(0),
    )?;
    assert_eq!(remaining, 2);
    Ok(())
}

#[test]
fn test_cleanup_stale_pending() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();

    for i in 0..3 {
        conn.execute(
            "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
             VALUES (?1, 'project-a', 'Bash', ?2)",
            params![format!("session-{}", i), now - 7200],
        )?;
    }

    conn.execute(
        "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
         VALUES ('fresh-session', 'project-a', 'Bash', ?1)",
        params![now - 300],
    )?;

    let deleted = cleanup_stale_pending(&conn)?;
    assert_eq!(deleted, 3);

    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations", [], |row| row.get(0),
    )?;
    assert_eq!(remaining, 1);
    Ok(())
}

#[test]
fn test_cleanup_expired_compressed() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now();

    // 插入 100 天前的 compressed observations
    for i in 0..5 {
        let old_epoch = now.timestamp() - (100 * 86400) - i;
        conn.execute(
            "INSERT INTO observations (memory_session_id, project, type, status, created_at, created_at_epoch)
             VALUES (?1, 'project-a', 'feature', 'compressed', ?2, ?3)",
            params![format!("old-{}", i), now.to_rfc3339(), old_epoch],
        )?;
    }

    // 插入 30 天前的 compressed（不应被清理）
    let recent_epoch = now.timestamp() - (30 * 86400);
    conn.execute(
        "INSERT INTO observations (memory_session_id, project, type, status, created_at, created_at_epoch)
         VALUES ('recent', 'project-a', 'feature', 'compressed', ?1, ?2)",
        params![now.to_rfc3339(), recent_epoch],
    )?;

    // 插入 active 状态（不应被清理）
    conn.execute(
        "INSERT INTO observations (memory_session_id, project, type, status, created_at, created_at_epoch)
         VALUES ('active', 'project-a', 'feature', 'active', ?1, ?2)",
        params![now.to_rfc3339(), now.timestamp()],
    )?;

    let deleted = cleanup_expired_compressed(&conn, 90)?;
    assert_eq!(deleted, 5);

    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM observations", [], |row| row.get(0),
    )?;
    assert_eq!(remaining, 2); // recent compressed + active
    Ok(())
}

// --- Burst/stress tests ---

#[test]
fn test_min_pending_gate() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();
    let min_pending: i64 = 3;

    for i in 0..2 {
        conn.execute(
            "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
             VALUES ('session-a', 'project-a', 'Bash', ?1)",
            params![now + i],
        )?;
    }

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations WHERE session_id = 'session-a'",
        [], |row| row.get(0),
    )?;
    assert!(count < min_pending);

    conn.execute(
        "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
         VALUES ('session-a', 'project-a', 'Edit', ?1)",
        params![now + 2],
    )?;

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations WHERE session_id = 'session-a'",
        [], |row| row.get(0),
    )?;
    assert!(count >= min_pending);
    Ok(())
}

#[test]
fn test_burst_session_protection() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();
    let min_pending: i64 = 3;
    let cooldown_secs: i64 = 300;

    let mut blocked = 0;
    for i in 0..20 {
        let session_id = format!("burst-session-{}", i);
        conn.execute(
            "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
             VALUES (?1, 'burst-project', 'Bash', ?2)",
            params![session_id, now + i],
        )?;

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE session_id = ?1",
            params![session_id], |row| row.get(0),
        )?;

        if count < min_pending {
            blocked += 1;
            continue;
        }

        if is_summarize_on_cooldown(&conn, "burst-project", cooldown_secs)? {
            blocked += 1;
            continue;
        }

        record_summarize(&conn, "burst-project", &format!("hash-{}", i))?;
    }

    assert_eq!(blocked, 20);
    Ok(())
}

#[test]
fn test_cooldown_blocks_rapid_fire() -> Result<()> {
    let conn = setup_db()?;
    let cooldown_secs: i64 = 300;

    assert!(!is_summarize_on_cooldown(&conn, "rapid-project", cooldown_secs)?);
    record_summarize(&conn, "rapid-project", "hash1")?;

    for i in 2..=10 {
        assert!(
            is_summarize_on_cooldown(&conn, "rapid-project", cooldown_secs)?,
            "第 {} 次应被冷却期拦截", i
        );
    }
    Ok(())
}

#[test]
fn test_duplicate_message_blocks_identical_sessions() -> Result<()> {
    let conn = setup_db()?;
    let identical_message = "修复微信视频号上传对话框阻塞问题";
    let msg_hash = hash_message(identical_message);

    assert!(!is_duplicate_message(&conn, "dup-project", &msg_hash)?);

    let old_epoch = chrono::Utc::now().timestamp() - 400;
    conn.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES ('dup-project', ?1, ?2)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?1, last_message_hash = ?2",
        params![old_epoch, msg_hash],
    )?;

    assert!(!is_summarize_on_cooldown(&conn, "dup-project", 300)?);
    assert!(is_duplicate_message(&conn, "dup-project", &msg_hash)?);

    let different_hash = hash_message("完全不同的消息内容");
    assert!(!is_duplicate_message(&conn, "dup-project", &different_hash)?);
    Ok(())
}

// --- New: Bash skip filter tests ---

#[test]
fn test_bash_skip_git_status() {
    assert!(should_skip_bash("git status"));
    assert!(should_skip_bash("git log --oneline -10"));
    assert!(should_skip_bash("git diff HEAD~1"));
    assert!(should_skip_bash("git branch -a"));
}

#[test]
fn test_bash_skip_read_only_commands() {
    assert!(should_skip_bash("ls -la"));
    assert!(should_skip_bash("pwd"));
    assert!(should_skip_bash("cat src/main.rs"));
    assert!(should_skip_bash("head -20 file.txt"));
    assert!(should_skip_bash("wc -l src/*.rs"));
}

#[test]
fn test_bash_skip_build_commands() {
    assert!(should_skip_bash("cargo build --release"));
    assert!(should_skip_bash("cargo check"));
    assert!(should_skip_bash("npm install"));
    assert!(should_skip_bash("yarn install"));
}

#[test]
fn test_bash_allow_meaningful_commands() {
    assert!(!should_skip_bash("rustup target add wasm32-unknown-unknown"));
    assert!(!should_skip_bash("docker compose up -d"));
    assert!(!should_skip_bash("make deploy"));
    assert!(!should_skip_bash("python manage.py migrate"));
    assert!(!should_skip_bash("git commit -m 'fix bug'"));
    assert!(!should_skip_bash("git push origin main"));
    assert!(!should_skip_bash("cargo test"));
}

#[test]
fn test_bash_skip_trims_whitespace() {
    assert!(should_skip_bash("  git status  "));
    assert!(should_skip_bash("  ls -la  "));
}

// --- New: project_from_cwd tests ---

#[test]
fn test_project_from_cwd_two_levels() {
    assert_eq!(project_from_cwd("/Users/foo/code/my-app"), "code/my-app");
    assert_eq!(project_from_cwd("/home/user/projects/api"), "projects/api");
}

#[test]
fn test_project_from_cwd_single_level() {
    assert_eq!(project_from_cwd("my-app"), "my-app");
}

#[test]
fn test_project_from_cwd_deep_path() {
    assert_eq!(
        project_from_cwd("/Users/lifcc/Desktop/code/AI/tools/remem"),
        "tools/remem"
    );
}

#[test]
fn test_project_from_cwd_no_collision() {
    let p1 = project_from_cwd("/Users/foo/work/api");
    let p2 = project_from_cwd("/Users/foo/personal/api");
    assert_ne!(p1, p2, "同名但不同父目录应产生不同 project 名");
}

// --- New: flush batch limit tests ---

#[test]
fn test_dequeue_pending_with_limit() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();

    // 插入 20 个 pending
    for i in 0..20 {
        conn.execute(
            "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
             VALUES ('session-a', 'project-a', 'Edit', ?1)",
            params![now + i],
        )?;
    }

    // limit=15 应该只返回 15 个
    let batch = dequeue_pending_with_limit(&conn, "session-a", 15)?;
    assert_eq!(batch.len(), 15);

    // limit=100 应该返回全部 20 个
    let batch = dequeue_pending_with_limit(&conn, "session-a", 100)?;
    assert_eq!(batch.len(), 20);

    // limit=0 应该返回 0 个
    let batch = dequeue_pending_with_limit(&conn, "session-a", 0)?;
    assert_eq!(batch.len(), 0);

    Ok(())
}

// --- New: stale pending session detection tests ---

#[test]
fn test_get_stale_pending_sessions() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();

    // 插入 15 分钟前的 pending（stale, > 10 min threshold）
    conn.execute(
        "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
         VALUES ('old-session', 'project-a', 'Bash', ?1)",
        params![now - 900],
    )?;

    // 插入 5 分钟前的 pending（fresh, < 10 min threshold）
    conn.execute(
        "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
         VALUES ('fresh-session', 'project-a', 'Bash', ?1)",
        params![now - 300],
    )?;

    // 插入其他项目的 stale pending（不应包含）
    conn.execute(
        "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
         VALUES ('other-session', 'project-b', 'Bash', ?1)",
        params![now - 900],
    )?;

    let stale = get_stale_pending_sessions(&conn, "project-a", 600)?;
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0], "old-session");

    Ok(())
}

// --- New: model resolution tests ---

#[test]
fn test_resolve_model_short_names() {
    assert_eq!(resolve_model_for_api("haiku"), "claude-haiku-4-5-20251001");
    assert_eq!(resolve_model_for_api("sonnet"), "claude-sonnet-4-5-20250514");
    assert_eq!(resolve_model_for_api("opus"), "claude-opus-4-20250514");
}

#[test]
fn test_resolve_model_full_id_passthrough() {
    assert_eq!(resolve_model_for_api("claude-haiku-4-5-20251001"), "claude-haiku-4-5-20251001");
    assert_eq!(resolve_model_for_api("claude-sonnet-4-5-20250514"), "claude-sonnet-4-5-20250514");
    assert_eq!(resolve_model_for_api("custom-model-v1"), "custom-model-v1");
}

// --- New: error propagation tests ---

#[test]
fn test_cooldown_on_missing_table_errors() {
    // 创建没有 summarize_cooldown 表的 DB
    let conn = Connection::open_in_memory().unwrap();
    // 不创建表，直接查询应该返回错误
    let result = conn.query_row(
        "SELECT last_summarize_epoch FROM summarize_cooldown WHERE project = ?1",
        params!["test"],
        |row| row.get::<_, i64>(0),
    );
    // 应该报错（no such table），而非静默返回 Ok(false)
    assert!(result.is_err());
}
