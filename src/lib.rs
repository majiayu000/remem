#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::manual_div_ceil
)]

pub mod adapter;
#[deprecated(note = "use remem::adapter::claude instead")]
pub mod adapter_claude {
    pub use crate::adapter::claude::*;
}
#[deprecated(note = "use remem::adapter::codex instead")]
pub mod adapter_codex {
    pub use crate::adapter::codex::*;
}
pub mod ai;
pub mod api;
pub mod claude_memory;
pub mod cli;
pub mod context;
pub mod db;
pub mod dedup;
pub mod doctor;
pub mod dream;
pub mod entity;
pub mod eval_local;
pub mod eval_metrics;
pub mod git_util;
pub mod identity;
pub mod install;
pub mod log;
pub mod mcp;
pub mod memory;
pub mod memory_format;
pub mod memory_promote;
pub mod memory_search;
pub mod memory_service;
pub mod migrate;
pub mod observe;
pub mod pending_admin;
pub mod preference;
pub mod project_id;
pub mod query_expand;
pub mod raw_archive;
pub mod search;
pub mod search_multihop;
pub mod summarize;
pub mod temporal;
pub mod timeline;
pub mod v2;
#[deprecated(note = "use remem::v2::db instead")]
pub mod v2_db {
    pub use crate::v2::db::*;
}
#[deprecated(note = "use remem::v2::gate instead")]
pub mod v2_gate {
    pub use crate::v2::gate::*;
}
#[deprecated(note = "use remem::v2::import instead")]
pub mod v2_import {
    pub use crate::v2::import::*;
}
#[deprecated(note = "use remem::v2::status instead")]
pub mod v2_status {
    pub use crate::v2::status::*;
}
pub mod vector;
pub mod worker;
pub mod workstream;

#[cfg(test)]
#[allow(deprecated)]
mod legacy_v2_module_path_tests {
    use std::path::{Path, PathBuf};

    #[test]
    fn legacy_v2_module_paths_still_compile() {
        let _default_path: fn() -> PathBuf = crate::v2_db::default_v2_db_path;
        let _detect_kind: fn(&rusqlite::Connection) -> anyhow::Result<crate::v2_gate::DbKind> =
            crate::v2_gate::detect_db_kind;
        let _import_legacy: fn(
            &Path,
            &rusqlite::Connection,
        ) -> anyhow::Result<crate::v2_import::ImportStats> =
            crate::v2_import::import_legacy_memories;
        let _format_summary: fn() -> Vec<String> = crate::v2_status::format_v2_summary;
    }
}
