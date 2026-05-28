pub(super) fn project_filter_sql(param_idx: usize) -> String {
    crate::retrieval::memory_search::project_or_global_clause("m.project", param_idx)
}

pub(super) fn branch_filter_sql(param_idx: usize) -> String {
    format!("(m.branch = ?{param_idx} OR m.branch IS NULL)")
}

pub(super) fn status_filter_sql(include_inactive: bool) -> String {
    crate::memory::memory_status_filter_sql("m.status", include_inactive)
}
