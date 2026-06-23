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
            valid_to_epoch INTEGER,
            identity_key TEXT,
            merged_into_workstream_id INTEGER
        );
        CREATE TABLE workstream_sessions (
            id INTEGER PRIMARY KEY,
            workstream_id INTEGER NOT NULL,
            memory_session_id TEXT NOT NULL,
            linked_at_epoch INTEGER NOT NULL,
            UNIQUE(workstream_id, memory_session_id)
        );
        CREATE TABLE workstream_aliases (
            id INTEGER PRIMARY KEY,
            workstream_id INTEGER NOT NULL,
            title TEXT NOT NULL,
            normalized_title TEXT NOT NULL,
            first_seen_epoch INTEGER NOT NULL,
            last_seen_epoch INTEGER NOT NULL,
            UNIQUE(workstream_id, normalized_title)
        );
        CREATE TABLE workstream_alias_sources (
            id INTEGER PRIMARY KEY,
            alias_id INTEGER NOT NULL,
            source TEXT NOT NULL,
            memory_session_id TEXT,
            source_workstream_id INTEGER,
            observed_title TEXT NOT NULL,
            first_seen_epoch INTEGER NOT NULL,
            last_seen_epoch INTEGER NOT NULL
        );",
    )
    .unwrap();
}
