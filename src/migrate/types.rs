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
        version: 18,
        name: "commit_session_links",
        sql: include_str!("../migrations/v018_commit_session_links.sql"),
    },
];

pub(crate) const OLD_BASELINE_VERSION: i64 = 13;

pub(crate) struct DryRunResult {
    pub current_version: i64,
    pub pending_count: usize,
    pub error: Option<String>,
}
