pub(crate) struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "baseline",
        sql: include_str!("../migrations/v001_baseline.sql"),
    },
    Migration {
        version: 2,
        name: "raw_messages",
        sql: include_str!("../migrations/v002_raw_messages.sql"),
    },
    Migration {
        version: 3,
        name: "host_identity",
        sql: include_str!("../migrations/v003_host_identity.sql"),
    },
    Migration {
        version: 4,
        name: "worker_heartbeat",
        sql: include_str!("../migrations/v004_worker_heartbeat.sql"),
    },
    Migration {
        version: 5,
        name: "memories_fts_active_filter",
        sql: include_str!("../migrations/v005_memories_fts_active_filter.sql"),
    },
    Migration {
        version: 6,
        name: "capture_pipeline",
        sql: include_str!("../migrations/v006_capture_pipeline.sql"),
    },
    Migration {
        version: 7,
        name: "session_rollup_ranges",
        sql: include_str!("../migrations/v007_session_rollup_ranges.sql"),
    },
    Migration {
        version: 8,
        name: "observation_evidence",
        sql: include_str!("../migrations/v008_observation_evidence.sql"),
    },
    Migration {
        version: 9,
        name: "memory_candidate_promotion",
        sql: include_str!("../migrations/v009_memory_candidate_promotion.sql"),
    },
    Migration {
        version: 10,
        name: "ai_usage_token_breakdown",
        sql: include_str!("../migrations/v010_ai_usage_token_breakdown.sql"),
    },
    Migration {
        version: 11,
        name: "reprice_ai_usage_events",
        sql: include_str!("../migrations/v011_reprice_ai_usage_events.sql"),
    },
    Migration {
        version: 12,
        name: "memory_search_context",
        sql: include_str!("../migrations/v012_memory_search_context.sql"),
    },
    Migration {
        version: 13,
        name: "memory_temporal_facts",
        sql: include_str!("../migrations/v013_memory_temporal_facts.sql"),
    },
    Migration {
        version: 14,
        name: "procedure_verifications",
        sql: include_str!("../migrations/v014_procedure_verifications.sql"),
    },
    Migration {
        version: 15,
        name: "rebuild_memory_search_context",
        sql: include_str!("../migrations/v015_rebuild_memory_search_context.sql"),
    },
    Migration {
        version: 16,
        name: "context_injection_gate",
        sql: include_str!("../migrations/v016_context_injection_gate.sql"),
    },
    Migration {
        version: 17,
        name: "memory_lessons",
        sql: include_str!("../migrations/v017_memory_lessons.sql"),
    },
    Migration {
        version: 18,
        name: "commit_session_links",
        sql: include_str!("../migrations/v018_commit_session_links.sql"),
    },
    Migration {
        version: 19,
        name: "memory_ownership",
        sql: include_str!("../migrations/v019_memory_ownership.sql"),
    },
    Migration {
        version: 20,
        name: "memory_fts_all_status",
        sql: include_str!("../migrations/v020_memory_fts_all_status.sql"),
    },
    Migration {
        version: 21,
        name: "raw_messages_session_dedup",
        sql: include_str!("../migrations/v021_raw_messages_session_dedup.sql"),
    },
    Migration {
        version: 22,
        name: "memory_state_keys",
        sql: include_str!("../migrations/v022_memory_state_keys.sql"),
    },
    Migration {
        version: 23,
        name: "topic_segments",
        sql: include_str!("../migrations/v023_topic_segments.sql"),
    },
    Migration {
        version: 24,
        name: "memory_operation_log",
        sql: include_str!("../migrations/v024_memory_operation_log.sql"),
    },
    Migration {
        version: 25,
        name: "memory_edges",
        sql: include_str!("../migrations/v025_memory_edges.sql"),
    },
    Migration {
        version: 26,
        name: "memory_claims",
        sql: include_str!("../migrations/v026_memory_claims.sql"),
    },
    Migration {
        version: 27,
        name: "compressed_observation_sources",
        sql: include_str!("../migrations/v027_compressed_observation_sources.sql"),
    },
    Migration {
        version: 28,
        name: "raw_ingest_failures",
        sql: include_str!("../migrations/v028_raw_ingest_failures.sql"),
    },
    Migration {
        version: 29,
        name: "memory_embeddings",
        sql: include_str!("../migrations/v029_memory_embeddings.sql"),
    },
    Migration {
        version: 30,
        name: "dream_cluster_decisions",
        sql: include_str!("../migrations/v030_dream_cluster_decisions.sql"),
    },
    Migration {
        version: 31,
        name: "graph_edges",
        sql: include_str!("../migrations/v031_graph_edges.sql"),
    },
    Migration {
        version: 32,
        name: "candidate_block_reason",
        sql: include_str!("../migrations/v032_candidate_block_reason.sql"),
    },
    Migration {
        version: 33,
        name: "graph_candidates",
        sql: include_str!("../migrations/v033_graph_candidates.sql"),
    },
    Migration {
        version: 34,
        name: "graph_edge_file_nodes",
        sql: include_str!("../migrations/v034_graph_edge_file_nodes.sql"),
    },
    Migration {
        version: 35,
        name: "context_injection_data_version",
        sql: include_str!("../migrations/v035_context_injection_data_version.sql"),
    },
    Migration {
        version: 36,
        name: "capture_drop_events",
        sql: include_str!("../migrations/v036_capture_drop_events.sql"),
    },
    Migration {
        version: 37,
        name: "graph_edge_source_candidate_integrity",
        sql: include_str!("../migrations/v037_graph_edge_source_candidate_integrity.sql"),
    },
    Migration {
        version: 38,
        name: "extraction_replay_ranges",
        sql: include_str!("../migrations/v038_extraction_replay_ranges.sql"),
    },
    Migration {
        version: 39,
        name: "context_injection_items",
        sql: include_str!("../migrations/v039_context_injection_items.sql"),
    },
    Migration {
        version: 40,
        name: "memory_fact_invalidations",
        sql: include_str!("../migrations/v040_memory_fact_invalidations.sql"),
    },
    Migration {
        version: 41,
        name: "content_identity_sha256",
        sql: include_str!("../migrations/v041_content_identity_sha256.sql"),
    },
    Migration {
        version: 42,
        name: "reference_time_epoch",
        sql: include_str!("../migrations/v042_reference_time_epoch.sql"),
    },
    Migration {
        version: 43,
        name: "graph_candidate_prompt_memory_refs",
        sql: include_str!("../migrations/v043_graph_candidate_prompt_memory_refs.sql"),
    },
    Migration {
        version: 44,
        name: "memory_embeddings_profile_index",
        sql: include_str!("../migrations/v044_memory_embeddings_profile_index.sql"),
    },
    Migration {
        version: 45,
        name: "memory_usage_columns",
        sql: include_str!("../migrations/v045_memory_usage_columns.sql"),
    },
    Migration {
        version: 46,
        name: "ai_usage_session_id",
        sql: include_str!("../migrations/v046_ai_usage_session_id.sql"),
    },
    Migration {
        version: 47,
        name: "lesson_outcome_metadata",
        sql: include_str!("../migrations/v047_lesson_outcome_metadata.sql"),
    },
    Migration {
        version: 48,
        name: "failure_lesson_feed_events",
        sql: include_str!("../migrations/v048_failure_lesson_feed_events.sql"),
    },
    Migration {
        version: 49,
        name: "user_context_claims",
        sql: include_str!("../migrations/v049_user_context_claims.sql"),
    },
    Migration {
        version: 50,
        name: "user_context_summaries",
        sql: include_str!("../migrations/v050_user_context_summaries.sql"),
    },
    Migration {
        version: 51,
        name: "memory_suppressions_feedback",
        sql: include_str!("../migrations/v051_memory_suppressions_feedback.sql"),
    },
    Migration {
        version: 52,
        name: "user_context_candidates",
        sql: include_str!("../migrations/v052_user_context_candidates.sql"),
    },
    Migration {
        version: 53,
        name: "workstream_identity_continuity",
        sql: include_str!("../migrations/v053_workstream_identity_continuity.sql"),
    },
    Migration {
        version: 54,
        name: "memory_candidate_source_kind",
        sql: include_str!("../migrations/v054_memory_candidate_source_kind.sql"),
    },
    Migration {
        version: 55,
        name: "session_ingest_cursors",
        sql: include_str!("../migrations/v055_session_ingest_cursors.sql"),
    },
    Migration {
        version: 56,
        name: "raw_messages_source_root_key",
        sql: include_str!("../migrations/v056_raw_messages_source_root_key.sql"),
    },
    Migration {
        version: 57,
        name: "failure_lifecycle",
        sql: include_str!("../migrations/v057_failure_lifecycle.sql"),
    },
    Migration {
        version: 58,
        name: "memory_embeddings_multimodel_key",
        sql: include_str!("../migrations/v058_memory_embeddings_multimodel_key.sql"),
    },
    Migration {
        version: 59,
        name: "candidate_review_metadata",
        sql: include_str!("../migrations/v059_candidate_review_metadata.sql"),
    },
    Migration {
        version: 60,
        name: "memory_poisoning_defense",
        sql: include_str!("../migrations/v060_memory_poisoning_defense.sql"),
    },
    Migration {
        version: 61,
        name: "memory_poisoning_injection_drops",
        sql: include_str!("../migrations/v061_memory_poisoning_injection_drops.sql"),
    },
    Migration {
        version: 62,
        name: "preference_rule_state",
        sql: include_str!("../migrations/v062_preference_rule_state.sql"),
    },
    Migration {
        version: 63,
        name: "procedure_exports",
        sql: include_str!("../migrations/v063_procedure_exports.sql"),
    },
    Migration {
        version: 64,
        name: "reject_legacy_summary_jobs",
        sql: include_str!("../migrations/v064_reject_legacy_summary_jobs.sql"),
    },
    Migration {
        version: 65,
        name: "preference_reinforcement",
        sql: include_str!("../migrations/v065_preference_reinforcement.sql"),
    },
    Migration {
        version: 66,
        name: "session_rollup_evidence_checkpoint",
        sql: include_str!("../migrations/v066_session_rollup_evidence_checkpoint.sql"),
    },
    Migration {
        version: 67,
        name: "capture_git_evidence",
        sql: include_str!("../migrations/v067_capture_git_evidence.sql"),
    },
    Migration {
        version: 68,
        name: "session_rollup_followup_checkpoint",
        sql: include_str!("../migrations/v068_session_rollup_followup_checkpoint.sql"),
    },
    Migration {
        version: 69,
        name: "job_queue_atomicity",
        sql: include_str!("../migrations/v069_job_queue_atomicity.sql"),
    },
];

pub(crate) const OLD_BASELINE_VERSION: i64 = 13;

pub(crate) struct DryRunResult {
    pub migration_version: i64,
    pub sqlite_user_version: i64,
    pub current_version: i64,
    pub pending_count: usize,
    pub error: Option<String>,
}
