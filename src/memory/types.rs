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

    /// Map a raw observation_type (the legal observation vocabulary lives in
    /// `crate::db::models::OBSERVATION_TYPES`: bugfix/feature/refactor/discovery/
    /// decision/change) onto the candidate `MemoryType` it can support.
    ///
    /// The candidate vocabulary and the observation vocabulary are different
    /// word lists, so a raw string-equality comparison between them is wrong:
    /// `architecture` is a valid candidate type but never a valid observation
    /// type, so an architecture candidate could never be matched and could
    /// never auto-promote. `feature`/`refactor`/`change` observations all
    /// describe project discoveries, so they collapse onto `Discovery`.
    pub fn from_observation_type(observation_type: &str) -> Option<Self> {
        match observation_type.trim().to_ascii_lowercase().as_str() {
            "bugfix" => Some(Self::Bugfix),
            "decision" => Some(Self::Decision),
            "discovery" | "feature" | "refactor" | "change" => Some(Self::Discovery),
            _ => None,
        }
    }

    /// Whether an observation of `observation_type` can serve as supporting
    /// evidence for a candidate of `self`. Auto-promotable candidate types are
    /// matched to their observation equivalent; `Architecture` candidates have
    /// no observation equivalent, so they accept `Discovery`-class evidence
    /// (the closest project-knowledge observation class).
    pub fn supports_observation_type(self, observation_type: &str) -> bool {
        match Self::from_observation_type(observation_type) {
            Some(mapped) => {
                mapped == self || (self == Self::Architecture && mapped == Self::Discovery)
            }
            None => false,
        }
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

pub fn memory_current_filter_sql(
    status_column: &str,
    expires_column: &str,
    include_inactive: bool,
) -> String {
    if include_inactive {
        memory_status_filter_sql(status_column, true)
    } else {
        format!(
            "{status_column} = 'active' AND \
             ({expires_column} IS NULL OR {expires_column} > CAST(strftime('%s', 'now') AS INTEGER))"
        )
    }
}

pub fn memory_state_key_current_filter_sql(table_alias: &str) -> String {
    let table_alias = table_alias.trim();
    let qualifier = if table_alias.is_empty() {
        String::new()
    } else {
        format!("{table_alias}.")
    };
    format!(
        "({qualifier}state_key_id IS NULL OR NOT EXISTS (
             SELECT 1 FROM memory_state_keys sk
             WHERE sk.id = {qualifier}state_key_id
               AND sk.current_memory_id IS NOT NULL
               AND sk.current_memory_id <> {qualifier}id
         ))"
    )
}

pub fn memory_not_superseded_filter_sql(table_alias: &str) -> String {
    let table_alias = table_alias.trim();
    let qualifier = if table_alias.is_empty() {
        String::new()
    } else {
        format!("{table_alias}.")
    };
    format!(
        "NOT EXISTS (
             SELECT 1 FROM memory_edges supersede_edge
             WHERE supersede_edge.edge_type = 'supersedes'
               AND supersede_edge.from_memory_id = {qualifier}id
         )"
    )
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
    fn architecture_candidate_accepts_discovery_class_observations() {
        // architecture is a valid candidate type but never a valid observation
        // type; it must accept discovery-class observation evidence.
        assert!(MemoryType::Architecture.supports_observation_type("discovery"));
        assert!(MemoryType::Architecture.supports_observation_type("feature"));
        assert!(MemoryType::Architecture.supports_observation_type("refactor"));
        assert!(MemoryType::Architecture.supports_observation_type("change"));
        // but not unrelated classes
        assert!(!MemoryType::Architecture.supports_observation_type("bugfix"));
        assert!(!MemoryType::Architecture.supports_observation_type("decision"));
    }

    #[test]
    fn auto_promote_types_match_their_observation_equivalents() {
        assert!(MemoryType::Bugfix.supports_observation_type("bugfix"));
        assert!(MemoryType::Decision.supports_observation_type("decision"));
        assert!(MemoryType::Discovery.supports_observation_type("discovery"));
        // feature/refactor/change all collapse onto discovery
        assert!(MemoryType::Discovery.supports_observation_type("feature"));
        assert!(MemoryType::Discovery.supports_observation_type("refactor"));
        assert!(MemoryType::Discovery.supports_observation_type("change"));
        // mismatches stay false
        assert!(!MemoryType::Bugfix.supports_observation_type("decision"));
        assert!(!MemoryType::Decision.supports_observation_type("discovery"));
        // unknown observation type maps to nothing
        assert!(MemoryType::from_observation_type("architecture").is_none());
        assert!(MemoryType::from_observation_type("nonsense").is_none());
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
                reference_time_epoch INTEGER,
                status TEXT NOT NULL DEFAULT 'active',
                branch TEXT,
                scope TEXT DEFAULT 'project',
                last_accessed_epoch INTEGER,
                access_count INTEGER NOT NULL DEFAULT 0,
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
            CREATE TABLE memory_candidates (
                id INTEGER PRIMARY KEY
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
            CREATE INDEX idx_memory_embeddings_profile_memory_id
                ON memory_embeddings(model, dimensions, memory_id);
            CREATE TABLE context_injection_items (
                id INTEGER PRIMARY KEY,
                injection_run_id TEXT NOT NULL,
                host TEXT NOT NULL,
                project TEXT NOT NULL,
                session_id TEXT,
                injection_key TEXT NOT NULL,
                hook_source TEXT,
                context_hash TEXT,
                output_mode TEXT NOT NULL,
                decision TEXT NOT NULL,
                item_kind TEXT NOT NULL,
                item_id INTEGER,
                memory_id INTEGER,
                channel TEXT NOT NULL,
                score REAL,
                render_order INTEGER,
                status TEXT NOT NULL,
                drop_reason TEXT,
                title TEXT,
                provenance TEXT,
                staleness TEXT,
                injected_at_epoch INTEGER NOT NULL
            );
            CREATE TABLE memory_citation_events (
                id INTEGER PRIMARY KEY,
                host TEXT NOT NULL,
                project TEXT NOT NULL,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                message_hash TEXT NOT NULL,
                citation_line_present INTEGER NOT NULL DEFAULT 0,
                parsed_count INTEGER NOT NULL DEFAULT 0,
                matched_count INTEGER NOT NULL DEFAULT 0,
                inserted_count INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL,
                created_at_epoch INTEGER NOT NULL,
                UNIQUE(host, project, session_id, source, message_hash)
            );
            CREATE TABLE memory_usage_events (
                id INTEGER PRIMARY KEY,
                citation_event_id INTEGER NOT NULL,
                host TEXT NOT NULL,
                project TEXT NOT NULL,
                session_id TEXT NOT NULL,
                source TEXT NOT NULL,
                message_hash TEXT NOT NULL,
                memory_id INTEGER NOT NULL,
                context_injection_item_id INTEGER,
                created_at_epoch INTEGER NOT NULL,
                UNIQUE(host, project, session_id, source, message_hash, memory_id)
            );
            CREATE TABLE memory_operation_log (
                id INTEGER PRIMARY KEY,
                operation TEXT NOT NULL,
                planner_version TEXT NOT NULL,
                actor TEXT NOT NULL,
                source TEXT NOT NULL,
                owner_scope TEXT,
                owner_key TEXT,
                memory_type TEXT,
                state_key TEXT,
                input_topic_key TEXT,
                source_candidate_id INTEGER,
                result_memory_id INTEGER,
                superseded_ids TEXT NOT NULL DEFAULT '[]',
                conflicting_ids TEXT NOT NULL DEFAULT '[]',
                noop_reason TEXT,
                defer_reason TEXT,
                confidence REAL,
                reason TEXT,
                created_at_epoch INTEGER NOT NULL
            );
            CREATE INDEX idx_memory_operation_log_state
                ON memory_operation_log(owner_scope, owner_key, memory_type, state_key, created_at_epoch);
            CREATE TABLE memory_edges (
                id INTEGER PRIMARY KEY,
                edge_type TEXT NOT NULL,
                from_memory_id INTEGER,
                to_memory_id INTEGER,
                state_key_id INTEGER,
                source_candidate_id INTEGER,
                evidence_event_ids TEXT,
                source_operation_id INTEGER,
                confidence REAL,
                reason TEXT,
                created_at_epoch INTEGER NOT NULL,
                FOREIGN KEY(from_memory_id) REFERENCES memories(id),
                FOREIGN KEY(to_memory_id) REFERENCES memories(id),
                FOREIGN KEY(state_key_id) REFERENCES memory_state_keys(id),
                FOREIGN KEY(source_candidate_id) REFERENCES memory_candidates(id),
                FOREIGN KEY(source_operation_id) REFERENCES memory_operation_log(id)
            );
            CREATE INDEX idx_memory_edges_from
                ON memory_edges(from_memory_id, edge_type);
            CREATE INDEX idx_memory_edges_to
                ON memory_edges(to_memory_id, edge_type);
            CREATE INDEX idx_memory_edges_state
                ON memory_edges(state_key_id, edge_type, created_at_epoch);
            CREATE TABLE dream_cluster_decisions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project TEXT NOT NULL,
                memory_type TEXT NOT NULL,
                cluster_signature TEXT NOT NULL,
                decision TEXT NOT NULL CHECK(decision IN ('merged', 'no_merge', 'defer', 'failed')),
                reason TEXT,
                member_ids_json TEXT NOT NULL,
                cluster_size INTEGER NOT NULL,
                next_review_epoch INTEGER,
                source_memory_id INTEGER,
                source_operation_id INTEGER,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                last_seen_epoch INTEGER NOT NULL,
                UNIQUE(project, memory_type, cluster_signature),
                FOREIGN KEY(source_memory_id) REFERENCES memories(id),
                FOREIGN KEY(source_operation_id) REFERENCES memory_operation_log(id)
            );
            CREATE INDEX idx_dream_cluster_decisions_review
                ON dream_cluster_decisions(project, decision, next_review_epoch);
            CREATE INDEX idx_dream_cluster_decisions_signature
                ON dream_cluster_decisions(project, memory_type, cluster_signature);
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
                stale_after_epoch INTEGER,
                outcome_kind TEXT NOT NULL DEFAULT 'unknown'
                    CHECK (outcome_kind IN ('unknown', 'success', 'failure', 'recovery', 'correction', 'revert')),
                success_count INTEGER NOT NULL DEFAULT 0 CHECK (success_count >= 0),
                failure_count INTEGER NOT NULL DEFAULT 0 CHECK (failure_count >= 0),
                recovery_count INTEGER NOT NULL DEFAULT 0 CHECK (recovery_count >= 0),
                correction_count INTEGER NOT NULL DEFAULT 0 CHECK (correction_count >= 0),
                revert_count INTEGER NOT NULL DEFAULT 0 CHECK (revert_count >= 0)
            );",
        )
        .unwrap();
    }
}
