use anyhow::{Context, Result};
use rusqlite::{params, Connection};

pub(super) fn backfill_git_commit_files(conn: &Connection) -> Result<usize> {
    let mut select = conn.prepare(
        "SELECT id, sha, changed_files
         FROM git_commits
         ORDER BY id ASC",
    )?;
    let mut insert = conn.prepare(
        "INSERT OR IGNORE INTO git_commit_files(commit_id, path)
         VALUES (?1, ?2)",
    )?;
    let mut rows = select.query([])?;
    let mut inserted = 0;

    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let sha: String = row.get(1)?;
        let raw: String = row.get(2)?;
        let paths = serde_json::from_str::<Vec<String>>(&raw).with_context(|| {
            format!("parse git_commits.id={id} sha={sha} changed_files as a string array")
        })?;
        for path in paths {
            inserted += insert.execute(params![id, path])?;
        }
    }

    Ok(inserted)
}
