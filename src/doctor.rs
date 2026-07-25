mod capture_capability;
mod capture_liveness;
mod codex_native_memory;
mod cursor_install;
mod database;
mod embedding;
mod environment;
pub(crate) mod health_action;
mod logging;
mod mcp_processes;
mod memory_poisoning;
mod native_memory;
mod pack_imports;
mod procedure_exports;
mod promotion_funnel;
mod report;
mod reranker;
mod retrieval_enrichment;
mod review_queue;
mod rule_enforcement;
mod runtime_config_check;
mod schema;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use report::{run_doctor, DoctorOptions};
