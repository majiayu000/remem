pub(super) fn project_filter_sql(param_idx: usize) -> String {
    crate::memory_search::project_or_global_clause("m.project", param_idx)
}

pub(super) fn branch_filter_sql(param_idx: usize) -> String {
    format!("(m.branch = ?{param_idx} OR m.branch IS NULL)")
}

pub(super) fn status_filter_sql(include_inactive: bool) -> &'static str {
    if include_inactive {
        "1=1"
    } else {
        "m.status = 'active'"
    }
}
