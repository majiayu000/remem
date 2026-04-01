use crate::memory::Memory;
use crate::workstream::WorkStream;

#[derive(Debug, Clone)]
pub(super) struct SessionSummaryBrief {
    pub request: String,
    pub completed: Option<String>,
    pub created_at_epoch: i64,
}

#[derive(Debug)]
pub(super) struct LoadedContext {
    pub memories: Vec<Memory>,
    pub summaries: Vec<SessionSummaryBrief>,
    pub workstreams: Vec<WorkStream>,
}
