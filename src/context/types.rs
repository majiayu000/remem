use crate::memory::lesson::LessonMemory;
use crate::memory::Memory;
use crate::workstream::WorkStream;

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
    pub owner_traces: Vec<OwnerTrace>,
    pub owner_counts: OwnerCounts,
}
