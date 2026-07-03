pub(super) const MEMORY_TYPES: [&str; 4] = ["decision", "bugfix", "discovery", "lesson"];

pub(super) const FILE_PATHS: [&str; 8] = [
    "src/noise/cache_guard.rs",
    "src/noise/retry_plan.rs",
    "crates/noise_runtime/src/lib.rs",
    "apps/noise_panel/src/main.ts",
    "tests/noise_contract.rs",
    "docs/noise/runbook.md",
    "src/noise/vector_probe.rs",
    "src/noise/ledger_sink.rs",
];

pub(super) const CRATE_NAMES: [&str; 8] = [
    "aurora-cache",
    "brass-ledger",
    "cipher-ridge",
    "drift-panel",
    "ember-index",
    "frost-runner",
    "garnet-store",
    "harbor-signal",
];

pub(super) const ERROR_SIGNATURES: [&str; 8] = [
    "E_CAPACITY_001",
    "E_CAPACITY_017",
    "E_CAPACITY_029",
    "E_CAPACITY_041",
    "E_CAPACITY_053",
    "E_CAPACITY_067",
    "E_CAPACITY_079",
    "E_CAPACITY_083",
];

pub(super) const COMMANDS: [&str; 6] = [
    "cargo test noise_contract",
    "cargo run -- noise-probe",
    "node --test noise-runtime.test.js",
    "python3 scripts/noise_check.py",
    "cargo clippy --package noise-runtime",
    "remem eval --dataset eval/noise.json",
];

pub(super) const OWNERS: [&str; 6] = [
    "Ari Vale",
    "Bea Stone",
    "Cato Reed",
    "Dina Moss",
    "Eli Park",
    "Faye Holt",
];
