use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

pub(super) fn query_recent_commit_messages(
    conn: &Connection,
    project: &str,
    current_branch: Option<&str>,
    limit: usize,
) -> Result<Vec<String>> {
    let commits_table_exists = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'git_commits' LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if limit == 0 || !commits_table_exists {
        return Ok(vec![]);
    }

    let mut conditions = vec![
        "project = ?1".to_string(),
        "message IS NOT NULL".to_string(),
        "length(trim(message)) > 0".to_string(),
    ];
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(project.to_string())];
    let mut idx = 2;
    if let Some(branch) = current_branch.filter(|branch| !branch.trim().is_empty()) {
        conditions.push(format!("(branch = ?{idx} OR branch IS NULL)"));
        params.push(Box::new(branch.to_string()));
        idx += 1;
    }
    params.push(Box::new(limit as i64));

    let sql = format!(
        "SELECT message
         FROM git_commits
         WHERE {}
         ORDER BY COALESCE(authored_at_epoch, updated_at_epoch) DESC, id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&params);
    let rows = stmt.query_map(refs.as_slice(), |row| row.get::<_, String>(0))?;
    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|message| message.trim().to_string())
        .filter(|message| !message.is_empty())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_commit_messages_return_empty_when_table_is_absent() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;

        let messages = query_recent_commit_messages(&conn, "/tmp/remem", Some("main"), 3)?;

        assert!(messages.is_empty());
        Ok(())
    }

    #[test]
    fn recent_commit_messages_filter_project_and_branch() -> anyhow::Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE git_commits (
                id INTEGER PRIMARY KEY,
                project TEXT NOT NULL,
                repo_path TEXT NOT NULL,
                sha TEXT NOT NULL,
                short_sha TEXT NOT NULL,
                branch TEXT,
                message TEXT,
                authored_at_epoch INTEGER,
                changed_files TEXT NOT NULL DEFAULT '[]',
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL
            );",
        )?;
        insert_commit(&conn, 1, "/tmp/remem", None, "Keep branchless context", 10)?;
        insert_commit(
            &conn,
            2,
            "/tmp/remem",
            Some("feature/context"),
            "Wire context retrieval",
            30,
        )?;
        insert_commit(&conn, 3, "/tmp/other", Some("feature/context"), "Other", 40)?;
        insert_commit(&conn, 4, "/tmp/remem", Some("other"), "Wrong branch", 50)?;

        let messages =
            query_recent_commit_messages(&conn, "/tmp/remem", Some("feature/context"), 3)?;

        assert_eq!(
            messages,
            vec![
                "Wire context retrieval".to_string(),
                "Keep branchless context".to_string()
            ]
        );
        Ok(())
    }

    fn insert_commit(
        conn: &Connection,
        id: i64,
        project: &str,
        branch: Option<&str>,
        message: &str,
        updated_at_epoch: i64,
    ) -> anyhow::Result<()> {
        conn.execute(
            "INSERT INTO git_commits
             (id, project, repo_path, sha, short_sha, branch, message,
              authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?2, ?3, ?3, ?4, ?5, ?6, '[]', ?6, ?6)",
            rusqlite::params![
                id,
                project,
                format!("sha{id}"),
                branch,
                message,
                updated_at_epoch
            ],
        )?;
        Ok(())
    }
}
