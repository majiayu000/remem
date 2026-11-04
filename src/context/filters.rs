use anyhow::Result;
use rusqlite::Connection;

pub(super) fn push_owner_included_filter(
    conn: &Connection,
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<()> {
    let clauses = push_owner_value_clauses(conn, project, idx, params)?;
    conditions.push(clauses.included());
    Ok(())
}

/// Same rows as [`push_owner_included_filter`], plus a rowid prefilter so
/// SQLite reaches the project's memories through the owner, target and legacy
/// project indexes. The three-way OR alone makes the planner walk every
/// active memory through the status index, decrypting the whole table on each
/// context query (about 150ms per channel on a store with 100k memories).
pub(super) fn push_owner_included_filter_indexed(
    conn: &Connection,
    project: &str,
    table_ref: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<()> {
    let clauses = push_owner_value_clauses(conn, project, idx, params)?;
    conditions.push(clauses.candidate_ids(table_ref));
    conditions.push(clauses.included());
    Ok(())
}

struct OwnerValueClauses {
    owner: String,
    target: String,
    legacy: String,
}

impl OwnerValueClauses {
    fn included(&self) -> String {
        format!(
            "((owner_scope = 'repo' AND {}) \
              OR (owner_scope = 'repo' AND {}) \
              OR (owner_scope IS NULL AND {} \
                  AND COALESCE(scope, 'project') != 'global'))",
            self.owner, self.target, self.legacy
        )
    }

    // Each branch is a superset of its `included` counterpart and names only
    // its own indexed column, so the planner picks that column's index.
    fn candidate_ids(&self, table_ref: &str) -> String {
        format!(
            "{table_ref}.id IN (
                 SELECT id FROM memories WHERE owner_scope = 'repo' AND {}
                 UNION SELECT id FROM memories WHERE {}
                 UNION SELECT id FROM memories WHERE {})",
            self.owner, self.target, self.legacy
        )
    }
}

fn push_owner_value_clauses(
    conn: &Connection,
    project: &str,
    idx: &mut usize,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<OwnerValueClauses> {
    let (owner, next) =
        crate::project_alias::push_project_value_filter(conn, "owner_key", project, *idx, params)?;
    *idx = next;
    let (target, next) = crate::project_alias::push_project_value_filter(
        conn,
        "target_project",
        project,
        *idx,
        params,
    )?;
    *idx = next;
    let (legacy, next) =
        crate::project_alias::push_project_value_filter(conn, "project", project, *idx, params)?;
    *idx = next;
    Ok(OwnerValueClauses {
        owner,
        target,
        legacy,
    })
}

pub(super) fn push_owner_excluded_filter(
    conn: &Connection,
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<()> {
    let clauses = push_owner_value_clauses(conn, project, idx, params)?;
    conditions.push(format!("NOT {}", clauses.included()));
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
