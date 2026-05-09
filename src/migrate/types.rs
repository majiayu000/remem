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
];

pub(crate) const OLD_BASELINE_VERSION: i64 = 13;

pub(crate) struct DryRunResult {
    pub current_version: i64,
    pub pending_count: usize,
    pub error: Option<String>,
}
