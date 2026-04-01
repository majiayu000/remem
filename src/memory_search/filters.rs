/// Push exact project filter into SQL conditions.
/// Returns the next parameter index.
pub fn push_project_filter(
    column: &str,
    project: Option<&str>,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> usize {
    if let Some(project) = project {
        let (clause, next_idx) =
            crate::project_id::push_project_filter(column, project, idx, params);
        conditions.push(clause);
        idx = next_idx;
    }
    idx
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
