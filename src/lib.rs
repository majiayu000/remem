#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_div_ceil
)]

pub mod adapter;
pub mod adapter_claude;
pub mod ai;
pub mod api;
pub mod claude_memory;
pub mod context;
pub mod db;
pub mod db_job;
pub mod db_models;
pub mod db_pending;
pub mod db_query;
pub mod db_usage;
pub mod dedup;
pub mod doctor;
pub mod eval_metrics;
pub mod install;
pub mod log;
pub mod mcp;
pub mod memory;
pub mod memory_format;
pub mod memory_promote;
pub mod memory_search;
pub mod observe;
pub mod observe_flush;
pub mod preference;
pub mod query_expand;
pub mod search;
pub mod summarize;
pub mod timeline;
pub mod vector;
pub mod worker;
pub mod workstream;
