use rusqlite::types::ToSql;

pub(super) fn clamp_error(error: &str) -> String {
    crate::db::truncate_str(error, 1000).to_string()
}

pub(super) fn id_placeholders(ids: &[i64], start_idx: usize) -> String {
    (start_idx..start_idx + ids.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn append_ids(params: &mut Vec<Box<dyn ToSql>>, ids: &[i64]) {
    for id in ids {
        params.push(Box::new(*id));
    }
}
