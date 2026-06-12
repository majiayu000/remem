use crate::memory::lesson::LessonMemory;
use crate::memory::Memory;
use crate::workstream::WorkStream;
use std::fmt;

use super::host::HostKind;
use super::ownership::{OwnerCounts, OwnerTrace};

#[derive(Debug, Clone)]
pub(super) struct ContextRequest {
    pub cwd: String,
    pub project: String,
    pub session_id: Option<String>,
    pub hook_source: Option<String>,
    pub current_branch: Option<String>,
    pub host: HostKind,
    pub use_colors: bool,
}

#[derive(Debug, Clone)]
pub(super) struct SessionSummaryBrief {
    pub request: String,
    pub completed: Option<String>,
    pub created_at_epoch: i64,
}

#[derive(Debug)]
pub(super) struct LoadedContext {
    pub memories: Vec<Memory>,
    pub lessons: Vec<LessonMemory>,
    pub summaries: Vec<SessionSummaryBrief>,
    pub workstreams: Vec<WorkStream>,
    pub memory_abstained: bool,
    pub errors: Vec<ContextLoadError>,
    pub owner_traces: Vec<OwnerTrace>,
    pub owner_counts: OwnerCounts,
    pub diagnostics: ContextDiagnostics,
}

#[derive(Debug, Clone)]
pub(super) struct ContextLoadError {
    pub section: &'static str,
    pub message: String,
}

impl ContextLoadError {
    pub(super) fn new(section: &'static str, error: impl fmt::Display) -> Self {
        Self {
            section,
            message: error.to_string(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ContextDiagnostics {
    pub candidate_pool_total: usize,
    pub current_rows: usize,
    pub selected_ids: Vec<i64>,
    pub hidden_duplicate_groups: Vec<HiddenDuplicateGroup>,
    pub preference_selected_ids: Vec<i64>,
    pub preference_hidden_duplicate_groups: Vec<HiddenDuplicateGroup>,
    pub preference_state_key_groups: Vec<StateKeyDiagnosticGroup>,
    pub state_key_groups: Vec<StateKeyDiagnosticGroup>,
    pub exclusions: Vec<ContextExclusion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HiddenDuplicateGroup {
    pub cluster_key: String,
    pub chosen_id: i64,
    pub hidden_ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContextExclusion {
    pub id: i64,
    pub reason: &'static str,
    pub status: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StateKeyDiagnosticGroup {
    pub owner_scope: String,
    pub owner_key: String,
    pub memory_type: String,
    pub state_key: String,
    pub current_id: Option<i64>,
    pub active_ids: Vec<i64>,
    pub reason: &'static str,
}
