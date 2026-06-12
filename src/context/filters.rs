pub(super) fn push_owner_included_filter(
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    let owner_key_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    let target_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    let legacy_project_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    conditions.push(format!(
        "((owner_scope = 'repo' AND owner_key = ?{owner_key_idx}) \
          OR (owner_scope = 'repo' AND target_project = ?{target_idx}) \
          OR (owner_scope IS NULL AND project = ?{legacy_project_idx} \
              AND COALESCE(scope, 'project') != 'global'))"
    ));
}

pub(super) fn push_owner_excluded_filter(
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    let owner_key_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    let target_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    let legacy_project_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    conditions.push(format!(
        "NOT ((owner_scope = 'repo' AND owner_key = ?{owner_key_idx}) \
              OR (owner_scope = 'repo' AND target_project = ?{target_idx}) \
              OR (owner_scope IS NULL AND project = ?{legacy_project_idx} \
                  AND COALESCE(scope, 'project') != 'global'))"
    ));
}

pub(super) fn push_context_related_filter(
    project: &str,
    idx: &mut usize,
    conditions: &mut Vec<String>,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    let project_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    let source_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    let target_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    let owner_idx = *idx;
    params.push(Box::new(project.to_string()));
    *idx += 1;
    conditions.push(format!(
        "(project = ?{project_idx} OR source_project = ?{source_idx} \
          OR target_project = ?{target_idx} OR owner_key = ?{owner_idx})"
    ));
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
