use anyhow::Result;
use rusqlite::{params, Connection};

use remem::{db, memory, observe, search};

fn setup_observation_schema(conn: &Connection) -> Result<()> {
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

        CREATE TABLE observations (
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
            discovery_tokens INTEGER DEFAULT 0,
            created_at TEXT,
            created_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER,
            branch TEXT,
            commit_sha TEXT
        );

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
        END;",
    )?;
    Ok(())
}

fn setup_memory_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE memories (
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
            status TEXT NOT NULL DEFAULT 'active',
            branch TEXT,
            scope TEXT DEFAULT 'project'
        );

        CREATE VIRTUAL TABLE memories_fts USING fts5(
            title, content,
            content='memories',
            content_rowid='id',
            tokenize='trigram'
        );

        CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;

        CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
            INSERT INTO memories_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;

        CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
        END;
        CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY,
            canonical_name TEXT NOT NULL COLLATE NOCASE,
            entity_type TEXT,
            mention_count INTEGER DEFAULT 1,
            created_at_epoch INTEGER NOT NULL DEFAULT 0,
            UNIQUE(canonical_name)
        );
        CREATE TABLE IF NOT EXISTS memory_entities (
            memory_id INTEGER NOT NULL,
            entity_id INTEGER NOT NULL,
            PRIMARY KEY(memory_id, entity_id)
        );",
    )?;
    Ok(())
}

#[test]
fn bash_skip_filter_stays_in_observe_module() {
    use remem::adapter_claude::should_skip_bash_command;
    assert!(should_skip_bash_command("git status"));
    assert!(should_skip_bash_command("  ls -la  "));
    assert!(should_skip_bash_command("cargo build --release"));
    assert!(!should_skip_bash_command("git commit -m 'fix'"));
    assert!(!should_skip_bash_command("cargo test"));
}

#[test]
fn project_key_is_stable_and_collision_resistant() {
    let a = db::project_from_cwd("/tmp/work/api");
    let b = db::project_from_cwd("/tmp/personal/api");
    let expected_a = db::canonical_project_path("/tmp/work/api")
        .to_string_lossy()
        .to_string();
    let expected_b = db::canonical_project_path("/tmp/personal/api")
        .to_string_lossy()
        .to_string();
    assert_eq!(a, expected_a);
    assert_eq!(b, expected_b);
    assert_ne!(a, b);
    // Stability: same path always produces same key
    assert_eq!(a, db::project_from_cwd("/tmp/work/api"));
}

#[test]
fn get_observations_by_ids_respects_project_filter() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;

    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, created_at, created_at_epoch, status)
         VALUES (1, 'm1', 'p1', 'feature', 'one', '2026-02-21T00:00:00Z', 1700000000, 'active')",
        [],
    )?;
    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, created_at, created_at_epoch, status)
         VALUES (2, 'm2', 'p2', 'feature', 'two', '2026-02-21T00:00:00Z', 1700000001, 'active')",
        [],
    )?;

    let all = db::get_observations_by_ids(&conn, &[1, 2], None)?;
    assert_eq!(all.len(), 2);

    let filtered = db::get_observations_by_ids(&conn, &[1, 2], Some("p1"))?;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, 1);
    Ok(())
}

#[test]
fn search_handles_hyphenated_queries_without_fts_error() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;

    memory::insert_memory(
        &conn,
        Some("s1"),
        "p",
        None,
        "om-generator refactor",
        "CRE deal member refactoring complete",
        "discovery",
        None,
    )?;

    let results = search::search(&conn, Some("om-generator"), None, None, 10, 0, true)?;
    assert_eq!(results.len(), 1);

    let results = search::search(&conn, Some("om-generator CRE"), None, None, 10, 0, true)?;
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn search_decay_prefers_newer_records_on_same_match() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_observation_schema(&conn)?;
    let now = chrono::Utc::now().timestamp();
    let old = now - 60 * 86400;

    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, narrative, created_at, created_at_epoch, status)
         VALUES (?1, 'm1', 'p', 'feature', 'hello same', 'hello same', '2026-01-01T00:00:00Z', ?2, 'active')",
        params![1_i64, old],
    )?;
    conn.execute(
        "INSERT INTO observations
         (id, memory_session_id, project, type, title, narrative, created_at, created_at_epoch, status)
         VALUES (?1, 'm2', 'p', 'feature', 'hello same', 'hello same', '2026-02-21T00:00:00Z', ?2, 'active')",
        params![2_i64, now],
    )?;

    let results = db::search_observations_fts(&conn, "hello", Some("p"), None, 10, 0, true)?;
    assert!(results.len() >= 2);
    assert_eq!(results[0].id, 2);
    Ok(())
}

/// Helper to insert a test memory.
fn insert_mem(conn: &Connection, project: &str, title: &str, content: &str) -> Result<i64> {
    memory::insert_memory(
        conn,
        Some("s1"),
        project,
        None,
        title,
        content,
        "discovery",
        None,
    )
}

#[test]
fn search_chinese_4char_via_fts_trigram() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(&conn, "p", "竞品对标分析报告", "调研了多个内存框架")?;
    insert_mem(&conn, "p", "English only title", "No Chinese here")?;

    let results = search::search(&conn, Some("竞品对标"), None, None, 10, 0, true)?;
    assert_eq!(results.len(), 1);
    assert!(results[0].title.contains("竞品"));
    Ok(())
}

#[test]
fn search_chinese_2char_via_like_fallback() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(&conn, "p", "竞品对标分析报告", "调研了多个内存框架")?;
    insert_mem(&conn, "p", "English only title", "No Chinese here")?;

    let results = search::search(&conn, Some("竞品"), None, None, 10, 0, true)?;
    assert!(!results.is_empty(), "should find at least 1 result");
    assert!(
        results[0].title.contains("竞品"),
        "first result should be most relevant"
    );
    Ok(())
}

#[test]
fn search_chinese_single_char_via_like_fallback() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(&conn, "p", "竞品对标分析报告", "调研了多个内存框架")?;

    let results = search::search(&conn, Some("框"), None, None, 10, 0, true)?;
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn search_english_short_token_via_like_fallback() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(&conn, "p", "DB schema migration", "Updated AI model")?;
    insert_mem(&conn, "p", "Other topic entirely", "Nothing relevant")?;

    let results = search::search(&conn, Some("DB"), None, None, 10, 0, true)?;
    assert!(!results.is_empty(), "should find at least 1 result");
    assert!(
        results[0].title.contains("DB"),
        "first result should be most relevant"
    );
    Ok(())
}

#[test]
fn search_mixed_chinese_english() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(&conn, "p", "Letta 框架分析", "内存管理系统调研")?;
    insert_mem(&conn, "p", "Letta overview", "English description")?;
    insert_mem(&conn, "p", "其他框架", "不相关内容")?;

    let results = search::search(&conn, Some("Letta 框架"), None, None, 10, 0, true)?;
    assert!(!results.is_empty(), "should find at least 1 result");
    // With OR semantics + synonym expansion, both "Letta 框架分析" and "Letta overview" may match
    assert!(
        results[0].title.contains("Letta"),
        "first result should contain query term"
    );
    Ok(())
}

#[test]
fn search_no_results_returns_empty() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(&conn, "p", "Hello world", "Some content")?;

    let results = search::search(&conn, Some("不存在的内容"), None, None, 10, 0, true)?;
    assert!(results.is_empty());
    Ok(())
}

#[test]
fn search_chinese_in_narrative_field() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(
        &conn,
        "p",
        "English title",
        "这段叙述包含工作流状态追踪的设计决策",
    )?;

    let results = search::search(&conn, Some("工作流"), None, None, 10, 0, true)?;
    assert_eq!(results.len(), 1);
    Ok(())
}

#[test]
fn search_with_project_filter() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn)?;
    insert_mem(&conn, "proj-a", "竞品对标报告", "分析内容")?;
    insert_mem(&conn, "proj-b", "竞品对标报告", "分析内容")?;

    let results = search::search(&conn, Some("竞品对标"), Some("proj-a"), None, 10, 0, true)?;
    assert_eq!(results.len(), 1);
    Ok(())
}
