use rusqlite::Connection;

pub(super) fn setup_workstream_schema(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE workstreams (
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
            completed_at_epoch INTEGER,
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
            valid_to_epoch INTEGER
        );
        CREATE TABLE workstream_sessions (
            id INTEGER PRIMARY KEY,
            workstream_id INTEGER NOT NULL,
            memory_session_id TEXT NOT NULL,
            linked_at_epoch INTEGER NOT NULL,
            UNIQUE(workstream_id, memory_session_id)
        );",
    )
    .unwrap();
}
