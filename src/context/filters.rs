use anyhow::Result;
use rusqlite::Connection;

pub(super) fn push_owner_included_filter(
    conn: &Connection,
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<()> {
    let (owner_clause, next) =
        crate::project_alias::push_project_value_filter(conn, "owner_key", project, *idx, params)?;
    *idx = next;
    let (target_clause, next) = crate::project_alias::push_project_value_filter(
        conn,
        "target_project",
        project,
        *idx,
        params,
    )?;
    *idx = next;
    let (legacy_clause, next) =
        crate::project_alias::push_project_value_filter(conn, "project", project, *idx, params)?;
    *idx = next;
    conditions.push(format!(
        "((owner_scope = 'repo' AND {owner_clause}) \
          OR (owner_scope = 'repo' AND {target_clause}) \
          OR (owner_scope IS NULL AND {legacy_clause} \
              AND COALESCE(scope, 'project') != 'global'))"
    ));
    Ok(())
}

pub(super) fn push_owner_excluded_filter(
    conn: &Connection,
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<()> {
    let (owner_clause, next) =
        crate::project_alias::push_project_value_filter(conn, "owner_key", project, *idx, params)?;
    *idx = next;
    let (target_clause, next) = crate::project_alias::push_project_value_filter(
        conn,
        "target_project",
        project,
        *idx,
        params,
    )?;
    *idx = next;
    let (legacy_clause, next) =
        crate::project_alias::push_project_value_filter(conn, "project", project, *idx, params)?;
    *idx = next;
    conditions.push(format!(
        "NOT ((owner_scope = 'repo' AND {owner_clause}) \
              OR (owner_scope = 'repo' AND {target_clause}) \
              OR (owner_scope IS NULL AND {legacy_clause} \
                  AND COALESCE(scope, 'project') != 'global'))"
    ));
    Ok(())
}

pub(super) fn push_context_related_filter(
    conn: &Connection,
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<()> {
    let mut clauses = Vec::new();
    for column in ["project", "source_project", "target_project", "owner_key"] {
        let (clause, next) =
            crate::project_alias::push_project_value_filter(conn, column, project, *idx, params)?;
        *idx = next;
        clauses.push(clause);
    }
    conditions.push(format!("({})", clauses.join(" OR ")));
    Ok(())
}

pub(super) fn push_excluded_type_filter(
    excluded_types: &[&str],
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    if excluded_types.is_empty() {
        return;
    }
    let placeholders: Vec<String> = excluded_types
        .iter()
        .map(|memory_type| {
            let placeholder = format!("?{idx}");
            params.push(Box::new((*memory_type).to_string()));
            *idx += 1;
            placeholder
        })
        .collect();
    conditions.push(format!("memory_type NOT IN ({})", placeholders.join(", ")));
}
