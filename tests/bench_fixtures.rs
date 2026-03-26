use anyhow::Result;
use rusqlite::{params, Connection};

// ---------------------------------------------------------------------------
// Schema setup (mirrors production schema for in-memory testing)
// ---------------------------------------------------------------------------

pub fn setup_full_schema(conn: &Connection) -> Result<()> {
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

        CREATE TABLE IF NOT EXISTS memories (
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

        CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
            INSERT INTO memories_fts(rowid, title, content)
            VALUES (new.id, new.title, new.content);
        END;

        CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
        END;

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
            discovery_tokens INTEGER DEFAULT 0,
            created_at TEXT,
            created_at_epoch INTEGER,
            status TEXT DEFAULT 'active',
            last_accessed_epoch INTEGER,
            branch TEXT,
            commit_sha TEXT
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
        END;

        CREATE TABLE IF NOT EXISTS session_summaries (
            id INTEGER PRIMARY KEY,
            memory_session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            message_hash TEXT,
            request TEXT,
            completed TEXT,
            decisions TEXT,
            learned TEXT,
            next_steps TEXT,
            preferences TEXT,
            branch TEXT,
            created_at_epoch INTEGER NOT NULL,
            usage_tokens INTEGER DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS pending_observations (
            id INTEGER PRIMARY KEY,
            session_id TEXT NOT NULL,
            project TEXT NOT NULL,
            tool_name TEXT NOT NULL,
            tool_input TEXT,
            tool_response TEXT,
            cwd TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'pending',
            attempt_count INTEGER NOT NULL DEFAULT 0,
            next_retry_epoch INTEGER,
            last_error TEXT,
            lease_owner TEXT,
            lease_expires_epoch INTEGER
        );

        CREATE TABLE IF NOT EXISTS workstreams (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            title TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            next_action TEXT,
            blockers TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL
        );
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

// ---------------------------------------------------------------------------
// Fixture: simulated coding session events
// ---------------------------------------------------------------------------

pub struct SessionFixture {
    pub session_id: String,
    pub project: String,
    pub events: Vec<EventFixture>,
}

pub struct EventFixture {
    pub event_type: String,
    pub summary: String,
    pub detail: Option<String>,
    pub files: Option<String>,
}

/// Generate a realistic multi-session coding scenario.
pub fn coding_session_fixtures() -> Vec<SessionFixture> {
    vec![
        SessionFixture {
            session_id: "sess-001".into(),
            project: "tools/remem".into(),
            events: vec![
                EventFixture {
                    event_type: "file_read".into(),
                    summary: "Read src/search.rs to understand FTS5 query logic".into(),
                    detail: Some("Explored sanitize_fts_query and search_with_branch".into()),
                    files: Some("[\"src/search.rs\"]".into()),
                },
                EventFixture {
                    event_type: "file_edit".into(),
                    summary: "Added LIKE fallback for short token queries in search".into(),
                    detail: Some("Tokens < 3 chars now use LIKE instead of FTS5 MATCH".into()),
                    files: Some("[\"src/search.rs\"]".into()),
                },
                EventFixture {
                    event_type: "bash_run".into(),
                    summary: "cargo test -- search".into(),
                    detail: Some("All 8 search tests passed".into()),
                    files: None,
                },
                EventFixture {
                    event_type: "file_edit".into(),
                    summary: "Fixed time decay scoring to prefer recent memories".into(),
                    detail: Some("Changed decay curve: 7d=1.0, 30d=0.7, older=0.4".into()),
                    files: Some("[\"src/context.rs\"]".into()),
                },
                EventFixture {
                    event_type: "bash_run".into(),
                    summary: "cargo test".into(),
                    detail: Some("48 tests passed, 0 failures".into()),
                    files: None,
                },
            ],
        },
        SessionFixture {
            session_id: "sess-002".into(),
            project: "tools/remem".into(),
            events: vec![
                EventFixture {
                    event_type: "file_read".into(),
                    summary: "Read src/memory.rs for insert_memory_full signature".into(),
                    detail: None,
                    files: Some("[\"src/memory.rs\"]".into()),
                },
                EventFixture {
                    event_type: "file_edit".into(),
                    summary: "Added global scope field to Memory struct".into(),
                    detail: Some(
                        "New scope field: project (default) or global for cross-project sharing"
                            .into(),
                    ),
                    files: Some("[\"src/memory.rs\", \"src/db.rs\"]".into()),
                },
                EventFixture {
                    event_type: "bash_run".into(),
                    summary: "cargo check".into(),
                    detail: Some("Compilation successful".into()),
                    files: None,
                },
            ],
        },
        // Cross-project session
        SessionFixture {
            session_id: "sess-003".into(),
            project: "web/dashboard".into(),
            events: vec![
                EventFixture {
                    event_type: "file_edit".into(),
                    summary: "Created React dashboard component".into(),
                    detail: Some("New component for displaying analytics".into()),
                    files: Some("[\"src/components/Dashboard.tsx\"]".into()),
                },
                EventFixture {
                    event_type: "bash_run".into(),
                    summary: "pnpm test".into(),
                    detail: Some("12 tests passed".into()),
                    files: None,
                },
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// Memory seed data for search evaluation
// ---------------------------------------------------------------------------

pub struct MemorySeed {
    pub project: String,
    pub topic_key: Option<String>,
    pub title: String,
    pub content: String,
    pub memory_type: String,
    pub scope: String,
    pub age_days: i64,
    /// Whether this memory is considered "relevant" for search query "FTS5 search"
    pub relevant_to_fts_query: bool,
    /// Whether this memory is considered "relevant" for search query "time decay"
    pub relevant_to_decay_query: bool,
}

/// 30 memories: 10 relevant to "FTS5 search", 20 noise.
pub fn search_eval_memories() -> Vec<MemorySeed> {
    let mut seeds = Vec::new();

    // --- 10 relevant to "FTS5 search" ---
    let relevant = vec![
        (
            "FTS5 search LIKE fallback",
            "Added LIKE fallback for short tokens that FTS5 cannot handle",
        ),
        (
            "FTS5 trigram tokenizer setup",
            "Configured FTS5 with trigram tokenizer for Chinese character support in search",
        ),
        (
            "Search query sanitization",
            "Wrapped each token in quotes for FTS5 MATCH safety to handle special characters",
        ),
        (
            "FTS5 search ranking with time decay",
            "Search results now ranked by FTS5 relevance score multiplied by time decay factor",
        ),
        (
            "Memory search API endpoint",
            "REST API /search endpoint delegates to FTS5 search with project filter",
        ),
        (
            "FTS5 index rebuild after migration",
            "Schema v9 migration rebuilds FTS5 index to include new scope column",
        ),
        (
            "Search precision improvement",
            "Improved FTS5 search precision by using exact phrase matching for multi-word queries",
        ),
        (
            "Full-text search across observations",
            "Extended FTS5 search to cover observation title, subtitle, narrative, facts, concepts",
        ),
        (
            "Search result deduplication",
            "Deduplicated FTS5 search results by topic_key to avoid showing same memory twice",
        ),
        (
            "FTS5 search performance benchmark",
            "FTS5 search completes in under 5ms for 10k memories on M1 Mac",
        ),
    ];
    for (i, (title, content)) in relevant.iter().enumerate() {
        seeds.push(MemorySeed {
            project: "tools/remem".into(),
            topic_key: Some(format!("fts-rel-{}", i)),
            title: title.to_string(),
            content: content.to_string(),
            memory_type: if i % 3 == 0 { "decision" } else { "discovery" }.into(),
            scope: "project".into(),
            age_days: (i as i64) * 3,
            relevant_to_fts_query: true,
            relevant_to_decay_query: i == 3, // only one is also relevant to decay
        });
    }

    // --- 5 relevant to "time decay" ---
    let decay_relevant = vec![
        (
            "Time decay scoring formula",
            "Memory score = type_weight * time_decay where decay is 1.0/0.7/0.4 for 7d/30d/older",
        ),
        (
            "Context core memory selection",
            "Top 6 memories selected by score which includes time decay factor",
        ),
        (
            "Decay curve tuning decision",
            "Decided on 7/30 day thresholds for decay curve after testing with real session data",
        ),
        (
            "Time-based observation compression",
            "Old observations beyond 30 days compressed by AI to save context window budget",
        ),
        (
            "Recent memory boost in ranking",
            "Memories updated within 7 days get full weight 1.0 in time decay calculation",
        ),
    ];
    for (i, (title, content)) in decay_relevant.iter().enumerate() {
        seeds.push(MemorySeed {
            project: "tools/remem".into(),
            topic_key: Some(format!("decay-rel-{}", i)),
            title: title.to_string(),
            content: content.to_string(),
            memory_type: "decision".into(),
            scope: "project".into(),
            age_days: (i as i64) * 5,
            relevant_to_fts_query: false,
            relevant_to_decay_query: true,
        });
    }

    // --- 15 noise memories ---
    let noise = vec![
        (
            "Git branch detection",
            "Detect current git branch via git rev-parse for branch-scoped memories",
        ),
        (
            "SQLite WAL mode enabled",
            "Enabled WAL journal mode for better concurrent read performance",
        ),
        (
            "MCP server stdio transport",
            "Using rmcp crate for MCP stdio transport with JSON-RPC",
        ),
        (
            "Session summary cooldown",
            "300 second cooldown between summary generations for same project",
        ),
        (
            "Preference auto-promotion",
            "Preferences from session summaries auto-promoted to global scope",
        ),
        (
            "Docker build optimization",
            "Multi-stage Docker build reduces image size to 15MB",
        ),
        (
            "CI workflow with cargo test",
            "GitHub Actions CI runs cargo test on every push to main",
        ),
        (
            "Log rotation strategy",
            "Logs rotated daily with 7-day retention in ~/.remem/logs/",
        ),
        (
            "API rate limiting design",
            "Token bucket rate limiter for AI calls: 10 req/min per project",
        ),
        (
            "Database encryption setup",
            "SQLCipher encryption with key stored in ~/.remem/.key",
        ),
        (
            "Hook installation flow",
            "remem install patches Claude Code hooks.json to add observe/summarize hooks",
        ),
        (
            "Workstream auto-creation",
            "Workstreams auto-created from session summary request field",
        ),
        (
            "Memory deduplication logic",
            "Dedup by topic_key: same key updates existing instead of creating new",
        ),
        (
            "Observation flush pipeline",
            "Pending observations flushed to observation table via LLM extraction",
        ),
        (
            "REST API CORS configuration",
            "tower-http CORS layer allows all origins for local development",
        ),
    ];
    for (i, (title, content)) in noise.iter().enumerate() {
        seeds.push(MemorySeed {
            project: "tools/remem".into(),
            topic_key: Some(format!("noise-{}", i)),
            title: title.to_string(),
            content: content.to_string(),
            memory_type: "discovery".into(),
            scope: "project".into(),
            age_days: (i as i64) * 2 + 1,
            relevant_to_fts_query: false,
            relevant_to_decay_query: false,
        });
    }

    seeds
}

// ---------------------------------------------------------------------------
// Summary XML fixtures for parse/promote testing
// ---------------------------------------------------------------------------

pub fn summary_xml_with_all_fields() -> String {
    "<summary>\n\
     <request>Add global memory scope for cross-project preference sharing</request>\n\
     <completed>Implemented scope field on Memory struct, updated insert/query paths, \
     added auto-promotion of preferences to global scope</completed>\n\
     <decisions>Use 'global' scope string instead of boolean flag for extensibility. \
     Preferences are auto-promoted to global scope during summary processing.</decisions>\n\
     <learned>FTS5 trigram tokenizer handles Chinese characters well but requires \
     LIKE fallback for tokens under 3 characters</learned>\n\
     <next_steps>Add scope filter to MCP search tool. Write migration for existing \
     preferences to global scope.</next_steps>\n\
     <preferences>Always use English for code comments and commit messages. \
     Prefer cargo check before cargo test for faster feedback.</preferences>\n\
     </summary>"
        .to_string()
}

pub fn summary_xml_skip() -> String {
    "<skip_summary reason=\"trivial session\" />".to_string()
}

pub fn summary_xml_partial() -> String {
    "<summary>\n\
     <request>Fix search crash on empty query</request>\n\
     <completed>Added guard clause for empty/whitespace-only queries</completed>\n\
     </summary>"
        .to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub fn insert_seed_memories(conn: &Connection, seeds: &[MemorySeed]) -> Result<Vec<i64>> {
    let now = chrono::Utc::now().timestamp();
    let mut ids = Vec::new();
    for seed in seeds {
        let epoch = now - seed.age_days * 86400;
        conn.execute(
            "INSERT INTO memories \
             (session_id, project, topic_key, title, content, memory_type, \
              created_at_epoch, updated_at_epoch, status, scope) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 'active', ?8)",
            params![
                "bench-session",
                seed.project,
                seed.topic_key,
                seed.title,
                seed.content,
                seed.memory_type,
                epoch,
                seed.scope,
            ],
        )?;
        ids.push(conn.last_insert_rowid());
    }
    Ok(ids)
}

/// Insert a single memory with explicit epoch timestamp.
pub fn insert_memory_at(
    conn: &Connection,
    project: &str,
    title: &str,
    content: &str,
    memory_type: &str,
    epoch: i64,
    scope: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO memories \
         (session_id, project, title, content, memory_type, \
          created_at_epoch, updated_at_epoch, status, scope) \
         VALUES ('bench', ?1, ?2, ?3, ?4, ?5, ?5, 'active', ?6)",
        params![project, title, content, memory_type, epoch, scope],
    )?;
    Ok(conn.last_insert_rowid())
}
