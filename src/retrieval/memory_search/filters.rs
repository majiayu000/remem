/// Push memory project visibility filter into SQL conditions.
/// When a project is provided, memory queries use project + global overlay.
/// Returns the next parameter index.
pub fn push_project_filter(
    column: &str,
    project: Option<&str>,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> usize {
    if let Some(project) = project {
        conditions.push(project_or_global_clause(column, idx));
        params.push(Box::new(project.to_string()));
        idx += 1;
    }
    idx
}

pub fn push_project_filter_required(
    column: &str,
    project: &str,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> usize {
    conditions.push(project_or_global_clause(column, idx));
    params.push(Box::new(project.to_string()));
    idx += 1;
    idx
}

pub fn project_or_global_clause(column: &str, param_idx: usize) -> String {
    format!("({column} = ?{param_idx} OR scope = 'global')")
}

pub(super) fn push_branch_filter(
    column: &str,
    branch: Option<&str>,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> usize {
    if let Some(branch) = branch {
        conditions.push(format!("({column} = ?{idx} OR {column} IS NULL)"));
        params.push(Box::new(branch.to_string()));
        idx += 1;
    }
    idx
}
