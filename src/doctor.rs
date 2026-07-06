mod capture_capability;
mod capture_liveness;
mod database;
mod embedding;
mod environment;
pub(crate) mod health_action;
mod hook_validation;
mod logging;
mod mcp_processes;
mod memory_poisoning;
mod native_memory;
mod promotion_funnel;
mod report;
mod review_queue;
mod runtime_config_check;
mod schema;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use report::{run_doctor, DoctorOptions};
