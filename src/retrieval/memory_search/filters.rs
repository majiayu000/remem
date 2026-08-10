use anyhow::Result;
use rusqlite::Connection;

/// Push memory project visibility filter into SQL conditions.
/// When a project is provided, memory queries use project + global overlay.
/// Returns the next parameter index.
pub fn push_project_filter(
    conn: &Connection,
    column: &str,
    project: Option<&str>,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<usize> {
    if let Some(project) = project {
        let (project_clause, next) =
            crate::project_alias::push_project_value_filter(conn, column, project, idx, params)?;
        conditions.push(format!("({project_clause} OR scope = 'global')"));
        idx = next;
    }
    Ok(idx)
}

pub fn push_project_filter_required(
    conn: &Connection,
    column: &str,
    project: &str,
    mut idx: usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<usize> {
    let (project_clause, next) =
        crate::project_alias::push_project_value_filter(conn, column, project, idx, params)?;
    conditions.push(format!("({project_clause} OR scope = 'global')"));
    idx = next;
    Ok(idx)
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
