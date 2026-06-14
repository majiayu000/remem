use anyhow::Result;
use rusqlite::Connection;

pub fn setup_observation_schema(conn: &Connection) -> Result<()> {
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

pub fn setup_memory_schema(conn: &Connection) -> Result<()> {
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
            search_context TEXT,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            reference_time_epoch INTEGER,
            status TEXT NOT NULL DEFAULT 'active',
            branch TEXT,
            scope TEXT DEFAULT 'project',
            source_project TEXT,
            target_project TEXT,
            owner_scope TEXT,
            owner_key TEXT,
            topic_domain TEXT,
            routing_confidence REAL,
            routing_reason TEXT,
            context_class TEXT,
            expires_at_epoch INTEGER,
            valid_from_epoch INTEGER,
            valid_to_epoch INTEGER,
            state_key_id INTEGER
        );
        CREATE TABLE memory_state_keys (
            id INTEGER PRIMARY KEY,
            owner_scope TEXT NOT NULL,
            owner_key TEXT NOT NULL,
            memory_type TEXT NOT NULL,
            state_key TEXT NOT NULL,
            state_label TEXT,
            state_status TEXT NOT NULL DEFAULT 'active',
            current_memory_id INTEGER,
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            UNIQUE(owner_scope, owner_key, memory_type, state_key)
        );
        CREATE TABLE memory_embeddings (
            memory_id INTEGER PRIMARY KEY,
            embedding BLOB NOT NULL,
            dimensions INTEGER NOT NULL,
            model TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE
        );
        CREATE INDEX idx_memory_embeddings_model
            ON memory_embeddings(model, updated_at_epoch);

        CREATE VIRTUAL TABLE memories_fts USING fts5(
            title, content, search_context,
            content='memories',
            content_rowid='id',
            tokenize='trigram'
        );

        CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
            INSERT INTO memories_fts(rowid, title, content, search_context)
            SELECT new.id, new.title, new.content, COALESCE(new.search_context, '')
            WHERE new.status = 'active';
        END;

        CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
            SELECT 'delete', old.id, old.title, old.content, COALESCE(old.search_context, '')
            WHERE old.status = 'active';
            INSERT INTO memories_fts(rowid, title, content, search_context)
            SELECT new.id, new.title, new.content, COALESCE(new.search_context, '')
            WHERE new.status = 'active';
        END;

        CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
            INSERT INTO memories_fts(memories_fts, rowid, title, content, search_context)
            SELECT 'delete', old.id, old.title, old.content, COALESCE(old.search_context, '')
            WHERE old.status = 'active';
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
