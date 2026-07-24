use super::SchemaInvariant;

macro_rules! v068_session_summary_column {
    ($column:literal) => {
        SchemaInvariant::column(
            68,
            "session_rollup_followup_checkpoint",
            "session_summaries",
            $column,
        )
    };
}

pub(in crate::migrate) const V068_SCHEMA_INVARIANTS: &[SchemaInvariant] = &[
    v068_session_summary_column!("followup_scheduling_completed_at_epoch"),
    v068_session_summary_column!("followup_scheduling_state"),
    v068_session_summary_column!("followup_compress_job_id"),
    v068_session_summary_column!("followup_dream_disposition"),
    v068_session_summary_column!("followup_dream_job_id"),
];
