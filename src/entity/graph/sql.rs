pub(super) fn build_expand_sql(
    entity_count: usize,
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    include_inactive: bool,
) -> String {
    let entity_placeholders: Vec<String> = (1..=entity_count)
        .map(|index| format!("?{index}"))
        .collect();
    let mut conditions = vec![
        format!("me.entity_id IN ({})", entity_placeholders.join(", ")),
        status_filter_sql(include_inactive).to_string(),
    ];
    let mut idx = entity_count + 1;
    if project.is_some() {
        conditions.push(project_filter_sql(idx));
        idx += 1;
    }
    if memory_type.is_some() {
        conditions.push(format!("m.memory_type = ?{idx}"));
        idx += 1;
    }
    if branch.is_some() {
        conditions.push(branch_filter_sql(idx));
        idx += 1;
    }
    format!(
        "SELECT me.memory_id, COUNT(DISTINCT me.entity_id) as shared_count
         FROM memory_entities me
         JOIN memories m ON m.id = me.memory_id
         WHERE {}
         GROUP BY me.memory_id
         ORDER BY shared_count DESC
         LIMIT ?{}",
        conditions.join(" AND "),
        idx
    )
}

pub(super) fn build_expand_params(
    entity_ids: &[i64],
    project: Option<&str>,
    memory_type: Option<&str>,
    branch: Option<&str>,
    limit: i64,
) -> Vec<Box<dyn rusqlite::types::ToSql>> {
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = entity_ids
        .iter()
        .map(|id| Box::new(*id) as Box<dyn rusqlite::types::ToSql>)
        .collect();
    if let Some(project) = project {
        params_vec.push(Box::new(project.to_string()));
    }
    if let Some(memory_type) = memory_type {
        params_vec.push(Box::new(memory_type.to_string()));
    }
    if let Some(branch) = branch {
        params_vec.push(Box::new(branch.to_string()));
    }
    params_vec.push(Box::new(limit));
    params_vec
}

fn project_filter_sql(param_idx: usize) -> String {
    format!("m.project = ?{param_idx}")
}

fn branch_filter_sql(param_idx: usize) -> String {
    format!("(m.branch = ?{param_idx} OR m.branch IS NULL)")
}

fn status_filter_sql(include_inactive: bool) -> &'static str {
    if include_inactive {
        "1=1"
    } else {
        "m.status = 'active'"
    }
}
