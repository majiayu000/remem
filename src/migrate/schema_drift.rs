use anyhow::{bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};

use super::state::{applied_versions, has_migration_table};
use super::transition::add_column_if_missing;

const V022: i64 = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SchemaObject {
    Table(&'static str),
    Column {
        table: &'static str,
        column: &'static str,
    },
    Index(&'static str),
    Trigger(&'static str),
}

impl SchemaObject {
    fn describe(self) -> String {
        match self {
            SchemaObject::Table(table) => format!("table {table}"),
            SchemaObject::Column { table, column } => format!("column {table}.{column}"),
            SchemaObject::Index(index) => format!("index {index}"),
            SchemaObject::Trigger(trigger) => format!("trigger {trigger}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SchemaInvariant {
    pub version: i64,
    pub migration: &'static str,
    pub object: SchemaObject,
}

impl SchemaInvariant {
    const fn table(version: i64, migration: &'static str, name: &'static str) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Table(name),
        }
    }

    const fn column(
        version: i64,
        migration: &'static str,
        table: &'static str,
        column: &'static str,
    ) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Column { table, column },
        }
    }

    const fn index(version: i64, migration: &'static str, name: &'static str) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Index(name),
        }
    }

    const fn trigger(version: i64, migration: &'static str, name: &'static str) -> Self {
        Self {
            version,
            migration,
            object: SchemaObject::Trigger(name),
        }
    }

    fn label(self) -> String {
        format!("v{:03}_{}", self.version, self.migration)
    }
}

pub(super) const SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    SchemaInvariant::table(20, "memory_fts_all_status", "memories_fts"),
    SchemaInvariant::trigger(20, "memory_fts_all_status", "memories_ai"),
    SchemaInvariant::trigger(20, "memory_fts_all_status", "memories_ad"),
    SchemaInvariant::trigger(20, "memory_fts_all_status", "memories_au"),
    SchemaInvariant::table(21, "raw_messages_session_dedup", "raw_messages"),
    SchemaInvariant::column(
        21,
        "raw_messages_session_dedup",
        "raw_messages",
        "session_id",
    ),
    SchemaInvariant::trigger(21, "raw_messages_session_dedup", "raw_messages_ai"),
    SchemaInvariant::trigger(21, "raw_messages_session_dedup", "raw_messages_ad"),
    SchemaInvariant::trigger(21, "raw_messages_session_dedup", "raw_messages_au"),
    SchemaInvariant::table(22, "memory_state_keys", "memory_state_keys"),
    SchemaInvariant::column(22, "memory_state_keys", "memories", "state_key_id"),
    SchemaInvariant::column(22, "memory_state_keys", "memory_candidates", "state_key"),
    SchemaInvariant::column(
        22,
        "memory_state_keys",
        "memory_candidates",
        "state_key_confidence",
    ),
    SchemaInvariant::column(
        22,
        "memory_state_keys",
        "memory_candidates",
        "state_key_reason",
    ),
    SchemaInvariant::index(22, "memory_state_keys", "idx_memory_state_keys_owner"),
    SchemaInvariant::index(22, "memory_state_keys", "idx_memory_state_keys_current"),
    SchemaInvariant::index(22, "memory_state_keys", "idx_memories_state_key_id"),
    SchemaInvariant::index(22, "memory_state_keys", "idx_memory_candidates_state_key"),
    SchemaInvariant::table(23, "topic_segments", "topic_segments"),
    SchemaInvariant::index(23, "topic_segments", "idx_topic_segments_project_trace"),
    SchemaInvariant::index(23, "topic_segments", "idx_topic_segments_session"),
    SchemaInvariant::table(24, "memory_operation_log", "memory_operation_log"),
    SchemaInvariant::index(24, "memory_operation_log", "idx_memory_operation_log_state"),
    SchemaInvariant::table(25, "memory_edges", "memory_edges"),
    SchemaInvariant::index(25, "memory_edges", "idx_memory_edges_from"),
    SchemaInvariant::index(25, "memory_edges", "idx_memory_edges_to"),
    SchemaInvariant::index(25, "memory_edges", "idx_memory_edges_state"),
    SchemaInvariant::table(26, "memory_claims", "memory_claims"),
    SchemaInvariant::table(26, "memory_claims", "memory_candidate_noops"),
    SchemaInvariant::index(26, "memory_claims", "idx_memory_claims_session"),
    SchemaInvariant::index(26, "memory_claims", "idx_memory_claims_recent"),
    SchemaInvariant::index(26, "memory_claims", "idx_memory_claims_fingerprint"),
    SchemaInvariant::index(26, "memory_claims", "idx_memory_candidate_noops_claim"),
    SchemaInvariant::index(26, "memory_claims", "idx_memory_candidate_noops_project"),
    SchemaInvariant::table(
        27,
        "compressed_observation_sources",
        "compressed_observation_sources",
    ),
    SchemaInvariant::index(
        27,
        "compressed_observation_sources",
        "idx_compressed_observation_sources_compressed",
    ),
    SchemaInvariant::index(
        27,
        "compressed_observation_sources",
        "idx_compressed_observation_sources_source",
    ),
    SchemaInvariant::table(28, "raw_ingest_failures", "raw_ingest_failures"),
    SchemaInvariant::index(
        28,
        "raw_ingest_failures",
        "idx_raw_ingest_failures_project_recent",
    ),
    SchemaInvariant::index(28, "raw_ingest_failures", "idx_raw_ingest_failures_session"),
    SchemaInvariant::table(29, "memory_embeddings", "memory_embeddings"),
    SchemaInvariant::index(29, "memory_embeddings", "idx_memory_embeddings_model"),
    SchemaInvariant::table(30, "dream_cluster_decisions", "dream_cluster_decisions"),
    SchemaInvariant::index(
        30,
        "dream_cluster_decisions",
        "idx_dream_cluster_decisions_review",
    ),
    SchemaInvariant::index(
        30,
        "dream_cluster_decisions",
        "idx_dream_cluster_decisions_signature",
    ),
    SchemaInvariant::table(31, "graph_edges", "graph_edges"),
    SchemaInvariant::index(31, "graph_edges", "idx_graph_edges_from"),
    SchemaInvariant::index(31, "graph_edges", "idx_graph_edges_to"),
    SchemaInvariant::index(31, "graph_edges", "idx_graph_edges_type"),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_validate_source_events_insert",
    ),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_validate_source_events_update",
    ),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_validate_nodes_insert",
    ),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_validate_nodes_update",
    ),
    SchemaInvariant::trigger(34, "graph_edge_file_nodes", "graph_edges_memories_delete"),
    SchemaInvariant::trigger(34, "graph_edge_file_nodes", "graph_edges_entities_delete"),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_memory_facts_delete",
    ),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_captured_events_delete",
    ),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_topic_segments_delete",
    ),
    SchemaInvariant::trigger(31, "graph_edges", "graph_edges_memory_state_keys_delete"),
    SchemaInvariant::column(
        32,
        "candidate_block_reason",
        "memory_candidates",
        "auto_promote_block_reason",
    ),
    SchemaInvariant::table(33, "graph_candidates", "graph_candidates"),
    SchemaInvariant::index(33, "graph_candidates", "idx_graph_candidates_review"),
    SchemaInvariant::index(33, "graph_candidates", "idx_graph_candidates_project"),
    SchemaInvariant::index(33, "graph_candidates", "idx_graph_candidates_dedupe"),
    SchemaInvariant::table(34, "graph_edge_file_nodes", "graph_file_nodes"),
    SchemaInvariant::index(34, "graph_edge_file_nodes", "idx_graph_file_nodes_source"),
    SchemaInvariant::trigger(
        34,
        "graph_edge_file_nodes",
        "graph_edges_graph_file_nodes_delete",
    ),
    SchemaInvariant::column(
        35,
        "context_injection_data_version",
        "context_injections",
        "data_version",
    ),
    SchemaInvariant::index(
        35,
        "context_injection_data_version",
        "idx_workstreams_target_status",
    ),
    SchemaInvariant::index(
        35,
        "context_injection_data_version",
        "idx_session_summaries_target_created",
    ),
    SchemaInvariant::table(36, "capture_drop_events", "capture_drop_events"),
    SchemaInvariant::index(
        36,
        "capture_drop_events",
        "idx_capture_drop_events_reason_time",
    ),
    SchemaInvariant::index(
        36,
        "capture_drop_events",
        "idx_capture_drop_events_unrecovered_spill",
    ),
    SchemaInvariant::trigger(
        37,
        "graph_edge_source_candidate_integrity",
        "graph_edges_validate_source_candidate_insert",
    ),
    SchemaInvariant::trigger(
        37,
        "graph_edge_source_candidate_integrity",
        "graph_edges_validate_source_candidate_update",
    ),
    SchemaInvariant::trigger(
        37,
        "graph_edge_source_candidate_integrity",
        "graph_edges_memory_candidates_delete",
    ),
    SchemaInvariant::trigger(
        37,
        "graph_edge_source_candidate_integrity",
        "graph_edges_memory_candidates_update_id",
    ),
    SchemaInvariant::trigger(
        37,
        "graph_edge_source_candidate_integrity",
        "graph_edges_graph_candidates_delete",
    ),
    SchemaInvariant::trigger(
        37,
        "graph_edge_source_candidate_integrity",
        "graph_edges_graph_candidates_update_id",
    ),
    SchemaInvariant::trigger(
        37,
        "graph_edge_source_candidate_integrity",
        "graph_edges_memory_operation_provenance_update",
    ),
    SchemaInvariant::table(38, "extraction_replay_ranges", "extraction_replay_ranges"),
    SchemaInvariant::column(
        38,
        "extraction_replay_ranges",
        "extraction_tasks",
        "replay_range_id",
    ),
    SchemaInvariant::index(
        38,
        "extraction_replay_ranges",
        "idx_extraction_replay_ranges_status",
    ),
    SchemaInvariant::index(
        38,
        "extraction_replay_ranges",
        "idx_extraction_replay_ranges_project",
    ),
    SchemaInvariant::index(
        38,
        "extraction_replay_ranges",
        "idx_extraction_tasks_replay_range",
    ),
    SchemaInvariant::table(39, "context_injection_items", "context_injection_items"),
    SchemaInvariant::index(
        39,
        "context_injection_items",
        "idx_context_injection_items_session",
    ),
    SchemaInvariant::index(
        39,
        "context_injection_items",
        "idx_context_injection_items_memory",
    ),
    SchemaInvariant::index(
        39,
        "context_injection_items",
        "idx_context_injection_items_project",
    ),
    SchemaInvariant::column(
        40,
        "memory_fact_invalidations",
        "memory_facts",
        "invalidated_at_epoch",
    ),
    SchemaInvariant::index(
        40,
        "memory_fact_invalidations",
        "idx_memory_facts_invalidated",
    ),
    SchemaInvariant::column(
        42,
        "reference_time_epoch",
        "captured_events",
        "reference_time_epoch",
    ),
    SchemaInvariant::column(
        42,
        "reference_time_epoch",
        "observations",
        "reference_time_epoch",
    ),
    SchemaInvariant::column(
        42,
        "reference_time_epoch",
        "memories",
        "reference_time_epoch",
    ),
    SchemaInvariant::index(
        42,
        "reference_time_epoch",
        "idx_captured_events_project_reference_time",
    ),
    SchemaInvariant::index(
        42,
        "reference_time_epoch",
        "idx_observations_project_reference_time",
    ),
    SchemaInvariant::index(
        42,
        "reference_time_epoch",
        "idx_memories_project_reference_time",
    ),
    SchemaInvariant::column(
        43,
        "graph_candidate_prompt_memory_refs",
        "graph_candidates",
        "prompt_memory_ref_ids",
    ),
    SchemaInvariant::index(
        44,
        "memory_embeddings_profile_index",
        "idx_memory_embeddings_profile_memory_id",
    ),
    SchemaInvariant::column(
        45,
        "memory_usage_columns",
        "memories",
        "last_accessed_epoch",
    ),
    SchemaInvariant::column(45, "memory_usage_columns", "memories", "access_count"),
    SchemaInvariant::index(45, "memory_usage_columns", "idx_memories_usage"),
    SchemaInvariant::table(45, "memory_usage_columns", "memory_citation_events"),
    SchemaInvariant::table(45, "memory_usage_columns", "memory_usage_events"),
    SchemaInvariant::index(
        45,
        "memory_usage_columns",
        "idx_memory_citation_events_project_recent",
    ),
    SchemaInvariant::index(
        45,
        "memory_usage_columns",
        "idx_memory_usage_events_memory_recent",
    ),
    SchemaInvariant::column(46, "ai_usage_session_id", "ai_usage_events", "session_id"),
    SchemaInvariant::index(46, "ai_usage_session_id", "idx_ai_usage_session_created"),
    SchemaInvariant::column(
        47,
        "lesson_outcome_metadata",
        "memory_lessons",
        "outcome_kind",
    ),
    SchemaInvariant::column(
        47,
        "lesson_outcome_metadata",
        "memory_lessons",
        "success_count",
    ),
    SchemaInvariant::column(
        47,
        "lesson_outcome_metadata",
        "memory_lessons",
        "failure_count",
    ),
    SchemaInvariant::column(
        47,
        "lesson_outcome_metadata",
        "memory_lessons",
        "recovery_count",
    ),
    SchemaInvariant::column(
        47,
        "lesson_outcome_metadata",
        "memory_lessons",
        "correction_count",
    ),
    SchemaInvariant::column(
        47,
        "lesson_outcome_metadata",
        "memory_lessons",
        "revert_count",
    ),
    SchemaInvariant::index(47, "lesson_outcome_metadata", "idx_memory_lessons_outcome"),
    SchemaInvariant::table(
        48,
        "failure_lesson_feed_events",
        "memory_lesson_feed_events",
    ),
    SchemaInvariant::index(
        48,
        "failure_lesson_feed_events",
        "idx_memory_lesson_feed_events_project_recent",
    ),
    SchemaInvariant::index(
        48,
        "failure_lesson_feed_events",
        "idx_memory_lesson_feed_events_memory",
    ),
    SchemaInvariant::table(49, "user_context_claims", "user_context_claims"),
    SchemaInvariant::index(
        49,
        "user_context_claims",
        "idx_user_context_claims_owner_active",
    ),
    SchemaInvariant::index(
        49,
        "user_context_claims",
        "idx_user_context_claims_user_recent",
    ),
    SchemaInvariant::index(49, "user_context_claims", "idx_user_context_claims_status"),
    SchemaInvariant::table(50, "user_context_summaries", "user_context_summaries"),
    SchemaInvariant::index(
        50,
        "user_context_summaries",
        "idx_user_context_summaries_owner_active",
    ),
    SchemaInvariant::index(
        50,
        "user_context_summaries",
        "idx_user_context_summaries_user_recent",
    ),
    SchemaInvariant::table(51, "memory_suppressions_feedback", "memory_suppressions"),
    SchemaInvariant::index(
        51,
        "memory_suppressions_feedback",
        "idx_memory_suppressions_target_active",
    ),
    SchemaInvariant::index(
        51,
        "memory_suppressions_feedback",
        "idx_memory_suppressions_owner_active",
    ),
    SchemaInvariant::table(51, "memory_suppressions_feedback", "memory_feedback"),
    SchemaInvariant::index(
        51,
        "memory_suppressions_feedback",
        "idx_memory_feedback_target_recent",
    ),
    SchemaInvariant::index(
        51,
        "memory_suppressions_feedback",
        "idx_memory_feedback_context_item",
    ),
    SchemaInvariant::table(52, "user_context_candidates", "user_context_candidates"),
    SchemaInvariant::index(
        52,
        "user_context_candidates",
        "idx_user_context_candidates_inbox",
    ),
    SchemaInvariant::index(
        52,
        "user_context_candidates",
        "idx_user_context_candidates_user_recent",
    ),
    SchemaInvariant::index(
        52,
        "user_context_candidates",
        "idx_user_context_candidates_dedupe",
    ),
    SchemaInvariant::column(
        53,
        "workstream_identity_continuity",
        "workstreams",
        "identity_key",
    ),
    SchemaInvariant::column(
        53,
        "workstream_identity_continuity",
        "workstreams",
        "merged_into_workstream_id",
    ),
    SchemaInvariant::table(53, "workstream_identity_continuity", "workstream_aliases"),
    SchemaInvariant::table(
        53,
        "workstream_identity_continuity",
        "workstream_alias_sources",
    ),
    SchemaInvariant::index(
        53,
        "workstream_identity_continuity",
        "idx_workstream_aliases_lookup",
    ),
    SchemaInvariant::index(
        53,
        "workstream_identity_continuity",
        "idx_workstream_alias_sources_alias",
    ),
    SchemaInvariant::index(
        53,
        "workstream_identity_continuity",
        "idx_workstreams_identity_key",
    ),
    SchemaInvariant::index(
        53,
        "workstream_identity_continuity",
        "idx_workstreams_merged_into",
    ),
];

pub(crate) fn validate_schema_invariants(conn: &Connection) -> Result<Vec<String>> {
    if !has_migration_table(conn) {
        return Ok(Vec::new());
    }

    let applied = applied_versions(conn)?;
    missing_schema_invariants(conn, &applied)
}

pub(super) fn repair_known_schema_drift(conn: &Connection, applied: &[i64]) -> Result<Vec<String>> {
    let mut repaired = Vec::new();
    if applied.contains(&V022) {
        let missing = missing_v022_objects(conn)?;
        if !missing.is_empty() {
            repair_v022_memory_state_keys(conn)
                .context("repair v022_memory_state_keys schema drift")?;
            let still_missing = missing_v022_objects(conn)?;
            if !still_missing.is_empty() {
                bail!(
                    "repair v022_memory_state_keys schema drift incomplete: {}",
                    still_missing.join(", ")
                );
            }
            repaired.push(format!("v022_memory_state_keys ({})", missing.join(", ")));
        }
    }

    if applied.contains(&31) {
        let trigger = SchemaObject::Trigger("graph_edges_memory_state_keys_delete");
        if !schema_object_exists(conn, trigger)? {
            install_v031_state_delete_trigger(conn)
                .context("install v031 graph_edges memory_state_keys delete trigger")?;
            if schema_object_exists(conn, trigger)? {
                repaired.push(
                    "v031_graph_edges (trigger graph_edges_memory_state_keys_delete)".to_string(),
                );
            }
        }
    }

    let unresolved = missing_schema_invariants(conn, applied)?;
    if !unresolved.is_empty() {
        bail!(
            "schema drift requires manual repair: {}",
            unresolved.join("; ")
        );
    }
    Ok(repaired)
}

pub(super) fn install_v031_state_delete_trigger(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "graph_edges")? || !table_exists(conn, "memory_state_keys")? {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS graph_edges_memory_state_keys_delete
        AFTER DELETE ON memory_state_keys
        BEGIN
            DELETE FROM graph_edges
            WHERE (from_node_kind = 'state' AND from_node_id = OLD.id)
               OR (to_node_kind = 'state' AND to_node_id = OLD.id);
        END;",
    )?;
    Ok(())
}

fn missing_schema_invariants(conn: &Connection, applied: &[i64]) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for invariant in SCHEMA_INVARIANTS {
        if !applied.contains(&invariant.version) || schema_object_exists(conn, invariant.object)? {
            continue;
        }
        missing.push(format!(
            "{} marked applied but missing {}",
            invariant.label(),
            invariant.object.describe()
        ));
    }
    Ok(missing)
}

fn repair_v022_memory_state_keys(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_state_keys (
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
            UNIQUE(owner_scope, owner_key, memory_type, state_key),
            FOREIGN KEY(current_memory_id) REFERENCES memories(id)
        );",
    )?;

    add_column_if_missing(conn, "memories", "state_key_id", "INTEGER")?;
    add_column_if_missing(conn, "memory_candidates", "state_key", "TEXT")?;
    add_column_if_missing(conn, "memory_candidates", "state_key_confidence", "REAL")?;
    add_column_if_missing(conn, "memory_candidates", "state_key_reason", "TEXT")?;

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memory_state_keys_owner
            ON memory_state_keys(owner_scope, owner_key, memory_type, state_status);
        CREATE INDEX IF NOT EXISTS idx_memory_state_keys_current
            ON memory_state_keys(current_memory_id)
            WHERE current_memory_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_memories_state_key_id
            ON memories(state_key_id)
            WHERE state_key_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_memory_candidates_state_key
            ON memory_candidates(owner_scope, owner_key, memory_type, state_key)
            WHERE state_key IS NOT NULL;",
    )?;
    Ok(())
}

fn missing_v022_objects(conn: &Connection) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for invariant in SCHEMA_INVARIANTS
        .iter()
        .filter(|invariant| invariant.version == V022)
    {
        if !schema_object_exists(conn, invariant.object)? {
            missing.push(invariant.object.describe());
        }
    }
    Ok(missing)
}

fn schema_object_exists(conn: &Connection, object: SchemaObject) -> Result<bool> {
    match object {
        SchemaObject::Table(table) => table_exists(conn, table),
        SchemaObject::Column { table, column } => column_exists(conn, table, column),
        SchemaObject::Index(index) => index_exists(conn, index),
        SchemaObject::Trigger(trigger) => trigger_exists(conn, trigger),
    }
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn trigger_exists(conn: &Connection, trigger: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='trigger' AND name=?1",
            [trigger],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn index_exists(conn: &Connection, index: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1",
            [index],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
