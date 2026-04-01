use anyhow::Result;

use crate::db::Observation;

/// Expects columns: id, memory_session_id, type, title, subtitle, narrative,
/// facts, concepts, files_read, files_modified, discovery_tokens,
/// created_at, created_at_epoch, project, status, last_accessed_epoch,
/// content_session_id, branch, commit_sha
pub(crate) fn map_observation_row(row: &rusqlite::Row) -> rusqlite::Result<Observation> {
    Ok(Observation {
        id: row.get(0)?,
        memory_session_id: row.get(1)?,
        r#type: row.get(2)?,
        title: row.get(3)?,
        subtitle: row.get(4)?,
        narrative: row.get(5)?,
        facts: row.get(6)?,
        concepts: row.get(7)?,
        files_read: row.get(8)?,
        files_modified: row.get(9)?,
        discovery_tokens: row.get(10)?,
        created_at: row.get(11)?,
        created_at_epoch: row.get(12)?,
        project: row.get(13)?,
        status: row
            .get::<_, Option<String>>(14)?
            .unwrap_or_else(|| "active".to_string()),
        last_accessed_epoch: row.get(15)?,
        content_session_id: row.get(16)?,
        branch: row.get(17)?,
        commit_sha: row.get(18)?,
    })
}

/// 旧版 claude-mem 用毫秒 epoch，remem 用秒 epoch。
/// 秒级 epoch 当前 ~1.7e9，毫秒级 ~1.7e12。以 1e10 为分界线排除旧数据。
pub(crate) const EPOCH_SECS_ONLY: &str = "created_at_epoch < 10000000000";

pub(crate) fn obs_select_cols(table_ref: &str) -> String {
    format!(
        "{t}.id, {t}.memory_session_id, {t}.type, {t}.title, {t}.subtitle, {t}.narrative, \
         {t}.facts, {t}.concepts, {t}.files_read, {t}.files_modified, {t}.discovery_tokens, \
         {t}.created_at, {t}.created_at_epoch, {t}.project, {t}.status, {t}.last_accessed_epoch, \
         (SELECT s.content_session_id FROM sdk_sessions s \
          WHERE s.memory_session_id = {t}.memory_session_id LIMIT 1) AS content_session_id, \
         {t}.branch, {t}.commit_sha",
        t = table_ref
    )
}

pub fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

pub fn push_project_filter(
    column: &str,
    project: &str,
    idx: usize,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> (String, usize) {
    crate::project_id::push_project_filter(column, project, idx, params)
}
