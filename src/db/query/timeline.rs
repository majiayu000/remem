use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::Observation;

use super::shared::{map_observation_row, obs_select_cols, push_project_filter, EPOCH_SECS_ONLY};

pub fn get_timeline_around(
    conn: &Connection,
    anchor_id: i64,
    depth_before: i64,
    depth_after: i64,
    project: Option<&str>,
) -> Result<Vec<Observation>> {
    let anchor_sql = format!(
        "SELECT {} FROM observations WHERE id = ?1",
        obs_select_cols("observations")
    );
    let anchor: Observation =
        conn.query_row(&anchor_sql, params![anchor_id], map_observation_row)?;
    let epoch = anchor.created_at_epoch;

    let build_sql = |is_before: bool, project_filter: Option<&str>| -> String {
        let cmp = if is_before { "<" } else { ">" };
        let order = if is_before { "DESC" } else { "ASC" };
        let extra = project_filter
            .map(|filter| format!(" AND {filter}"))
            .unwrap_or_default();
        format!(
            "SELECT {} FROM observations \
             WHERE {} AND created_at_epoch {} ?1{} \
             ORDER BY created_at_epoch {} LIMIT ?2",
            obs_select_cols("observations"),
            EPOCH_SECS_ONLY,
            cmp,
            extra,
            order
        )
    };

    let mut result = Vec::new();

    for (is_before, depth) in [(true, depth_before), (false, depth_after)] {
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(epoch), Box::new(depth)];
        let project_filter = if let Some(project_name) = project {
            let (filter, _) = push_project_filter("project", project_name, 3, &mut params_vec);
            Some(filter)
        } else {
            None
        };
        let sql = build_sql(is_before, project_filter.as_deref());
        let mut stmt = conn.prepare(&sql)?;
        let refs = crate::db::to_sql_refs(&params_vec);
        let rows = stmt.query_map(refs.as_slice(), map_observation_row)?;
        for row in rows {
            result.push(row?);
        }
    }

    result.push(anchor);
    result.sort_by_key(|observation| observation.created_at_epoch);
    Ok(result)
}
