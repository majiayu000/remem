use anyhow::Result;
use rusqlite::Connection;

use super::types::TitleQualityReport;

const MAX_GOOD_TITLE_LEN: usize = 120;

pub(super) fn check_title_quality(conn: &Connection) -> Result<TitleQualityReport> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'",
        [],
        |row| row.get(0),
    )?;
    let bullet_prefix: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'active'
         AND (title LIKE '• %' OR title LIKE '- %' OR title LIKE '* %'
              OR title LIKE 'Preference: %')",
        [],
        |row| row.get(0),
    )?;
    let too_long: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM memories WHERE status = 'active' AND LENGTH(title) > {}",
            MAX_GOOD_TITLE_LEN
        ),
        [],
        |row| row.get(0),
    )?;

    let bullet_rate = if total > 0 {
        bullet_prefix as f64 / total as f64
    } else {
        0.0
    };
    Ok(TitleQualityReport {
        total,
        bullet_prefix,
        too_long,
        bullet_rate,
    })
}
