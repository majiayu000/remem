#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_div_ceil
)]

pub mod adapter;
pub mod ai;
pub mod api;
pub mod capture;
pub mod cli;
pub mod context;
pub mod db;
pub mod doctor;
pub mod dream;
pub mod eval;
pub mod extraction;
pub mod git_util;
pub mod identity;
pub mod install;
pub mod log;
pub mod mcp;
pub mod memory;
pub mod migrate;
pub mod observe;
pub mod project_id;
pub mod retrieval;
pub mod summarize;
pub mod timeline;
pub mod worker;
pub mod workstream;
