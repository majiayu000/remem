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
pub mod memory_service;
pub mod migrate;
pub mod observe;
pub mod pending_admin;
pub mod preference;
pub mod project_id;
pub mod raw_archive;
pub mod retrieval;
#[deprecated(note = "use remem::retrieval::memory_search instead")]
pub mod memory_search {
    pub use crate::retrieval::memory_search::*;
}
#[deprecated(note = "use remem::retrieval::query_expand instead")]
pub mod query_expand {
    pub use crate::retrieval::query_expand::*;
}
#[deprecated(note = "use remem::retrieval::search instead")]
pub mod search {
    pub use crate::retrieval::search::*;
}
#[deprecated(note = "use remem::retrieval::search_multihop instead")]
pub mod search_multihop {
    pub use crate::retrieval::search_multihop::*;
}
pub mod summarize;
#[deprecated(note = "use remem::retrieval::temporal instead")]
pub mod temporal {
    pub use crate::retrieval::temporal::*;
}
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

#[cfg(test)]
#[allow(deprecated)]
mod legacy_retrieval_module_path_tests {
    #[test]
    fn legacy_retrieval_module_paths_still_compile() {
        let _search: fn(
            &rusqlite::Connection,
            Option<&str>,
            Option<&str>,
            Option<&str>,
            i64,
            i64,
            bool,
        ) -> anyhow::Result<Vec<crate::memory::Memory>> = crate::search::search;
        let _multi_hop: fn(
            &rusqlite::Connection,
            &str,
            Option<&str>,
            i64,
        ) -> anyhow::Result<crate::search_multihop::MultiHopResult> =
            crate::search_multihop::search_multi_hop;
        let _expand_query: fn(&str) -> Vec<String> = crate::query_expand::expand_query;
        let _temporal: fn(&str) -> Option<crate::temporal::TemporalConstraint> =
            crate::temporal::extract_temporal;
        let _project_filter: fn(&str, usize) -> String =
            crate::memory_search::project_or_global_clause;
    }
}
