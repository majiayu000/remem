#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_div_ceil
)]

pub mod adapter;
pub mod ai;
pub mod api;
mod atomic_file;
mod build_info;
mod captured_git;
pub mod cli;
pub mod context;
pub mod db;
pub mod doctor;
pub mod dream;
pub mod eval;
mod extraction_worker;
mod git_evidence;
pub mod git_trace;
pub mod git_util;
mod graph_candidate;
mod hook_integrity;
mod hook_stdin;
pub mod identity;
pub mod ingest;
pub mod install;
pub mod log;
pub mod mcp;
pub mod memory;
mod memory_candidate;
pub mod migrate;
mod observation_extract;
pub mod observe;
pub mod perf;
pub mod project_id;
pub mod retrieval;
pub mod rules;
pub mod runtime_config;
mod session_rollup;
mod spill_queue;
pub mod summarize;
pub mod timeline;
pub mod user_context;
pub mod worker;
pub mod workstream;
