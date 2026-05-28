use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryType {
    Decision,
    Discovery,
    Bugfix,
    Architecture,
    Lesson,
    Preference,
    Procedure,
    SessionActivity,
}

impl MemoryType {
    pub const ALL: [Self; 8] = [
        Self::Decision,
        Self::Discovery,
        Self::Bugfix,
        Self::Architecture,
        Self::Lesson,
        Self::Preference,
        Self::Procedure,
        Self::SessionActivity,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Decision => "decision",
            Self::Discovery => "discovery",
            Self::Bugfix => "bugfix",
            Self::Architecture => "architecture",
            Self::Lesson => "lesson",
            Self::Preference => "preference",
            Self::Procedure => "procedure",
            Self::SessionActivity => "session_activity",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Decision => "Decisions",
            Self::Discovery => "Discoveries",
            Self::Bugfix => "Bug Fixes",
            Self::Architecture => "Architecture",
            Self::Lesson => "Lessons",
            Self::Preference => "Preferences",
            Self::Procedure => "Procedures",
            Self::SessionActivity => "Sessions",
        }
    }

    pub const fn index_order(self) -> Option<usize> {
        match self {
            Self::Decision => Some(0),
            Self::Bugfix => Some(1),
            Self::Architecture => Some(2),
            Self::Discovery => Some(3),
            Self::Procedure => Some(4),
            Self::SessionActivity => Some(5),
            Self::Lesson | Self::Preference => None,
        }
    }

    pub const fn is_indexed(self) -> bool {
        matches!(
            self,
            Self::Decision
                | Self::Bugfix
                | Self::Architecture
                | Self::Discovery
                | Self::Procedure
                | Self::SessionActivity
        )
    }

    pub const fn is_core(self) -> bool {
        matches!(
            self,
            Self::Bugfix | Self::Architecture | Self::Decision | Self::Discovery
        )
    }

    pub const fn weight(self) -> f64 {
        match self {
            Self::Bugfix => 3.0,
            Self::Architecture => 2.6,
            Self::Decision => 2.2,
            Self::Discovery => 1.8,
            Self::Lesson | Self::Preference | Self::Procedure | Self::SessionActivity => 0.0,
        }
    }

    pub const fn auto_promote(self) -> bool {
        matches!(
            self,
            Self::Architecture | Self::Bugfix | Self::Decision | Self::Discovery
        )
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|memory_type| memory_type.as_str() == value)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub session_id: Option<String>,
    pub project: String,
    pub topic_key: Option<String>,
    pub title: String,
    pub text: String,
    pub memory_type: String,
    pub files: Option<String>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_scope() -> String {
    "project".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub event_type: String,
    pub summary: String,
    pub detail: Option<String>,
    pub files: Option<String>,
    pub exit_code: Option<i32>,
    pub created_at_epoch: i64,
}

pub const MEMORY_TYPES: &[&str] = &[
    MemoryType::Decision.as_str(),
    MemoryType::Discovery.as_str(),
    MemoryType::Bugfix.as_str(),
    MemoryType::Architecture.as_str(),
    MemoryType::Lesson.as_str(),
    MemoryType::Preference.as_str(),
    MemoryType::Procedure.as_str(),
    MemoryType::SessionActivity.as_str(),
];

pub const MEMORY_COLS: &str = "id, session_id, project, topic_key, title, content, memory_type, \
                              files, created_at_epoch, updated_at_epoch, status, branch, scope";

pub fn memory_status_filter_sql(column: &str, include_inactive: bool) -> String {
    if include_inactive {
        format!("{column} IN ('active', 'stale', 'archived')")
    } else {
        format!("{column} = 'active'")
    }
}

pub fn map_memory_row_pub(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    map_memory_row(row)
}

pub(super) fn map_memory_row(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project: row.get(2)?,
        topic_key: row.get(3)?,
        title: row.get(4)?,
        text: row.get(5)?,
        memory_type: row.get(6)?,
        files: row.get(7)?,
        created_at_epoch: row.get(8)?,
        updated_at_epoch: row.get(9)?,
        status: row.get(10)?,
        branch: row.get(11)?,
        scope: row
            .get::<_, Option<String>>(12)?
            .unwrap_or_else(|| "project".to_string()),
    })
}

pub(super) fn map_event_row(row: &rusqlite::Row) -> rusqlite::Result<Event> {
    Ok(Event {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project: row.get(2)?,
        event_type: row.get(3)?,
        summary: row.get(4)?,
        detail: row.get(5)?,
        files: row.get(6)?,
        exit_code: row.get(7)?,
        created_at_epoch: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{MemoryType, MEMORY_TYPES};

    #[test]
    fn memory_types_are_derived_from_canonical_enum_order() {
        let canonical = MemoryType::ALL
            .iter()
            .copied()
            .map(MemoryType::as_str)
            .collect::<Vec<_>>();

        assert_eq!(MEMORY_TYPES, canonical.as_slice());
    }

    #[test]
    fn procedure_has_context_metadata() {
        let memory_type = MemoryType::Procedure;

        assert_eq!(memory_type.as_str(), "procedure");
        assert_eq!(memory_type.label(), "Procedures");
        assert_eq!(memory_type.index_order(), Some(4));
        assert!(memory_type.is_indexed());
        assert!(!memory_type.is_core());
        assert_eq!(memory_type.weight(), 0.0);
        assert!(!memory_type.auto_promote());
    }
}

#[cfg(test)]
pub mod tests_helper {
    use rusqlite::Connection;

    pub fn setup_memory_schema(conn: &Connection) {
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
                status TEXT NOT NULL DEFAULT 'active',
                branch TEXT,
                scope TEXT DEFAULT 'project'
            );
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
            CREATE TABLE events (
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
            );
            CREATE TABLE IF NOT EXISTS memory_lessons (
                memory_id INTEGER PRIMARY KEY,
                confidence REAL NOT NULL DEFAULT 0.7,
                reinforcement_count INTEGER NOT NULL DEFAULT 1,
                source_evidence TEXT,
                last_reinforced_at_epoch INTEGER NOT NULL,
                stale_after_epoch INTEGER
            );",
        )
        .unwrap();
    }
}
