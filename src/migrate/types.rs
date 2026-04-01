pub(crate) struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

pub(crate) const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "baseline",
    sql: include_str!("../migrations/v001_baseline.sql"),
}];

pub(crate) const OLD_BASELINE_VERSION: i64 = 13;

pub(crate) struct DryRunResult {
    pub current_version: i64,
    pub pending_count: usize,
    pub error: Option<String>,
}
