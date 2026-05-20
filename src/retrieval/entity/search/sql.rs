use crate::retrieval::memory_search::{project_visibility_clause, ProjectScopeFilter};

pub(super) fn project_filter_sql(param_idx: usize, scope_filter: ProjectScopeFilter) -> String {
    project_visibility_clause("m.project", "m.scope", param_idx, scope_filter)
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
