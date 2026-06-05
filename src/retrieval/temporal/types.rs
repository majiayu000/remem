/// Time expression parser for temporal-aware retrieval.
/// Extracts time constraints from queries like "yesterday", "上周", "3 days ago".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalConstraint {
    pub start_epoch: i64,
    pub end_epoch: i64,
    pub field: TemporalField,
}

/// Which time axis the temporal query should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalField {
    /// When the remembered fact/event happened.
    EventTime,
    /// When the memory row was last changed.
    UpdatedAt,
}

impl TemporalField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EventTime => "event_time",
            Self::UpdatedAt => "updated_at_epoch",
        }
    }
}
