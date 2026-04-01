/// Time expression parser for temporal-aware retrieval.
/// Extracts time constraints from queries like "yesterday", "上周", "3 days ago".
pub struct TemporalConstraint {
    pub start_epoch: i64,
    pub end_epoch: i64,
}
