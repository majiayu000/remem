//! 测试 summarize rate limiting: 冷却期、message hash 去重、最小 pending gate。

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

fn is_summarize_on_cooldown(conn: &Connection, project: &str, cooldown_secs: i64) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let result: Option<i64> = conn
        .query_row(
            "SELECT last_summarize_epoch FROM summarize_cooldown WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )
        .ok();
    match result {
        Some(last_epoch) => Ok(now - last_epoch < cooldown_secs),
        None => Ok(false),
    }
}

fn is_duplicate_message(conn: &Connection, project: &str, message_hash: &str) -> Result<bool> {
    let result: Option<String> = conn
        .query_row(
            "SELECT last_message_hash FROM summarize_cooldown WHERE project = ?1",
            params![project],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    match result {
        Some(prev_hash) => Ok(prev_hash == message_hash),
        None => Ok(false),
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

fn hash_message(msg: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    msg.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// --- 测试 ---

#[test]
fn test_cooldown_no_record() -> Result<()> {
    let conn = setup_db()?;
    // 无记录时不在冷却期
    assert!(!is_summarize_on_cooldown(&conn, "test-project", 300)?);
    Ok(())
}

#[test]
fn test_cooldown_within_period() -> Result<()> {
    let conn = setup_db()?;
    record_summarize(&conn, "test-project", "hash1")?;
    // 刚记录，应在冷却期内
    assert!(is_summarize_on_cooldown(&conn, "test-project", 300)?);
    Ok(())
}

#[test]
fn test_cooldown_expired() -> Result<()> {
    let conn = setup_db()?;
    // 手动插入过期记录
    let old_epoch = chrono::Utc::now().timestamp() - 400;
    conn.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES (?1, ?2, ?3)",
        params!["test-project", old_epoch, "old_hash"],
    )?;
    // 300 秒冷却期已过
    assert!(!is_summarize_on_cooldown(&conn, "test-project", 300)?);
    Ok(())
}

#[test]
fn test_cooldown_per_project() -> Result<()> {
    let conn = setup_db()?;
    record_summarize(&conn, "project-a", "hash1")?;
    // project-a 在冷却期，project-b 不在
    assert!(is_summarize_on_cooldown(&conn, "project-a", 300)?);
    assert!(!is_summarize_on_cooldown(&conn, "project-b", 300)?);
    Ok(())
}

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
    // 第二次记录覆盖第一次
    assert!(!is_duplicate_message(&conn, "test-project", "hash1")?);
    assert!(is_duplicate_message(&conn, "test-project", "hash2")?);
    Ok(())
}

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

#[test]
fn test_cleanup_orphan_summaries() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now();

    // 插入有对应 observation 的 summary
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

    // 插入无对应 observation 的孤立 summary（mem-* 前缀）
    for i in 0..5 {
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
             VALUES (?1, 'project-b', 'orphan', ?2, ?3)",
            params![format!("mem-orphan{}", i), now.to_rfc3339(), now.timestamp()],
        )?;
    }

    // 插入非 mem-* 前缀的 summary（不应被清理）
    conn.execute(
        "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
         VALUES ('uuid-session', 'project-c', 'not orphan', ?1, ?2)",
        params![now.to_rfc3339(), now.timestamp()],
    )?;

    let deleted = cleanup_orphan_summaries(&conn)?;
    assert_eq!(deleted, 5);

    // 验证保留了正确的记录
    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_summaries",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(remaining, 2); // mem-abcd1234 + uuid-session
    Ok(())
}

#[test]
fn test_cleanup_duplicate_summaries() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now();

    // 同 session+project 插入多条 summary
    for i in 0..5 {
        conn.execute(
            "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
             VALUES ('session-a', 'project-a', ?1, ?2, ?3)",
            params![format!("request-{}", i), now.to_rfc3339(), now.timestamp() + i],
        )?;
    }

    // 不同 session 各一条
    conn.execute(
        "INSERT INTO session_summaries (memory_session_id, project, request, created_at, created_at_epoch)
         VALUES ('session-b', 'project-a', 'single', ?1, ?2)",
        params![now.to_rfc3339(), now.timestamp()],
    )?;

    let deleted = cleanup_duplicate_summaries(&conn)?;
    assert_eq!(deleted, 4); // 保留 session-a 最新 1 条 + session-b 1 条

    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_summaries",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(remaining, 2);
    Ok(())
}

#[test]
fn test_cleanup_stale_pending() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();

    // 插入过期的 pending（2 小时前）
    for i in 0..3 {
        conn.execute(
            "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
             VALUES (?1, 'project-a', 'Bash', ?2)",
            params![format!("session-{}", i), now - 7200],
        )?;
    }

    // 插入新鲜的 pending（5 分钟前）
    conn.execute(
        "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
         VALUES ('fresh-session', 'project-a', 'Bash', ?1)",
        params![now - 300],
    )?;

    let deleted = cleanup_stale_pending(&conn)?;
    assert_eq!(deleted, 3);

    let remaining: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(remaining, 1);
    Ok(())
}

#[test]
fn test_min_pending_gate() -> Result<()> {
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();
    let min_pending: i64 = 3;

    // 只有 2 个 pending（低于 min_pending=3）
    for i in 0..2 {
        conn.execute(
            "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
             VALUES ('session-a', 'project-a', 'Bash', ?1)",
            params![now + i],
        )?;
    }

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations WHERE session_id = 'session-a'",
        [],
        |row| row.get(0),
    )?;
    assert!(count < min_pending, "pending={} 应低于 min_pending={}", count, min_pending);

    // 添加第 3 个，达到阈值
    conn.execute(
        "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
         VALUES ('session-a', 'project-a', 'Edit', ?1)",
        params![now + 2],
    )?;

    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pending_observations WHERE session_id = 'session-a'",
        [],
        |row| row.get(0),
    )?;
    assert!(count >= min_pending, "pending={} 应达到 min_pending={}", count, min_pending);
    Ok(())
}

#[test]
fn test_burst_session_protection() -> Result<()> {
    // 模拟短命 session 风暴场景：20 个 session 各有 1 个 pending
    let conn = setup_db()?;
    let now = chrono::Utc::now().timestamp();
    let min_pending: i64 = 3;
    let cooldown_secs: i64 = 300;

    let mut blocked = 0;
    for i in 0..20 {
        let session_id = format!("burst-session-{}", i);
        // 每个 session 只有 1 个 pending
        conn.execute(
            "INSERT INTO pending_observations (session_id, project, tool_name, created_at_epoch)
             VALUES (?1, 'burst-project', 'Bash', ?2)",
            params![session_id, now + i],
        )?;

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pending_observations WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;

        // Gate 1: min pending
        if count < min_pending {
            blocked += 1;
            continue;
        }

        // Gate 2: cooldown
        if is_summarize_on_cooldown(&conn, "burst-project", cooldown_secs)? {
            blocked += 1;
            continue;
        }

        // 通过了所有 gate
        record_summarize(&conn, "burst-project", &format!("hash-{}", i))?;
    }

    // 所有 20 个 session 都应被 min_pending gate 拦截
    assert_eq!(blocked, 20, "所有短命 session 都应被 min_pending 拦截");
    Ok(())
}

#[test]
fn test_cooldown_blocks_rapid_fire() -> Result<()> {
    // 模拟同项目快速连续 summarize
    let conn = setup_db()?;
    let cooldown_secs: i64 = 300;

    // 第 1 次：通过
    assert!(!is_summarize_on_cooldown(&conn, "rapid-project", cooldown_secs)?);
    record_summarize(&conn, "rapid-project", "hash1")?;

    // 第 2-10 次：全部被冷却期拦截
    for i in 2..=10 {
        assert!(
            is_summarize_on_cooldown(&conn, "rapid-project", cooldown_secs)?,
            "第 {} 次应被冷却期拦截",
            i
        );
    }
    Ok(())
}

#[test]
fn test_duplicate_message_blocks_identical_sessions() -> Result<()> {
    // 模拟相同 assistant message 的多个 session
    let conn = setup_db()?;
    let identical_message = "修复微信视频号上传对话框阻塞问题";
    let msg_hash = hash_message(identical_message);

    // 第 1 次：通过
    assert!(!is_duplicate_message(&conn, "dup-project", &msg_hash)?);

    // 设置冷却期已过（通过手动设置旧时间戳）
    let old_epoch = chrono::Utc::now().timestamp() - 400;
    conn.execute(
        "INSERT INTO summarize_cooldown (project, last_summarize_epoch, last_message_hash)
         VALUES ('dup-project', ?1, ?2)
         ON CONFLICT(project) DO UPDATE SET
           last_summarize_epoch = ?1, last_message_hash = ?2",
        params![old_epoch, msg_hash],
    )?;

    // 冷却期已过，但 message hash 相同 → 应被拦截
    assert!(!is_summarize_on_cooldown(&conn, "dup-project", 300)?);
    assert!(is_duplicate_message(&conn, "dup-project", &msg_hash)?);

    // 不同消息 → 不应被拦截
    let different_hash = hash_message("完全不同的消息内容");
    assert!(!is_duplicate_message(&conn, "dup-project", &different_hash)?);
    Ok(())
}
