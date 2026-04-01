use rusqlite::Connection;

pub(super) fn setup_entity_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE memories (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            title TEXT,
            text TEXT,
            memory_type TEXT NOT NULL DEFAULT 'discovery',
            branch TEXT,
            status TEXT NOT NULL DEFAULT 'active'
        );
        CREATE TABLE entities (
            id INTEGER PRIMARY KEY,
            canonical_name TEXT NOT NULL UNIQUE COLLATE NOCASE,
            entity_type TEXT,
            mention_count INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE memory_entities (
            memory_id INTEGER NOT NULL,
            entity_id INTEGER NOT NULL,
            UNIQUE(memory_id, entity_id)
        );",
    )
    .unwrap();
}
