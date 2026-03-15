use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkStreamStatus {
    Active,
    Paused,
    Completed,
    Abandoned,
}

impl WorkStreamStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn from_db(s: &str) -> Self {
        match s {
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "abandoned" => Self::Abandoned,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkStream {
    pub id: i64,
    pub project: String,
    pub title: String,
    pub description: Option<String>,
    pub status: WorkStreamStatus,
    pub progress: Option<String>,
    pub next_action: Option<String>,
    pub blockers: Option<String>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
    pub completed_at_epoch: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ParsedWorkStream {
    pub title: Option<String>,
    pub progress: Option<String>,
    pub next_action: Option<String>,
    pub blockers: Option<String>,
    pub is_completed: bool,
}

pub fn query_active_workstreams(conn: &Connection, project: &str) -> Result<Vec<WorkStream>> {
    let mut stmt = conn.prepare(
        "SELECT id, project, title, description, status, progress, next_action, blockers,
                created_at_epoch, updated_at_epoch, completed_at_epoch
         FROM workstreams
         WHERE project = ?1 AND status IN ('active', 'paused')
         ORDER BY updated_at_epoch DESC",
    )?;
    let rows = stmt.query_map(params![project], |row| {
        let status_str: String = row.get(4)?;
        Ok(WorkStream {
            id: row.get(0)?,
            project: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            status: WorkStreamStatus::from_db(&status_str),
            progress: row.get(5)?,
            next_action: row.get(6)?,
            blockers: row.get(7)?,
            created_at_epoch: row.get(8)?,
            updated_at_epoch: row.get(9)?,
            completed_at_epoch: row.get(10)?,
        })
    })?;
    crate::db_query::collect_rows(rows)
}

pub fn query_workstreams(
    conn: &Connection,
    project: &str,
    status_filter: Option<&str>,
) -> Result<Vec<WorkStream>> {
    let (sql, filter_val);
    if let Some(status) = status_filter {
        sql = "SELECT id, project, title, description, status, progress, next_action, blockers,
                      created_at_epoch, updated_at_epoch, completed_at_epoch
               FROM workstreams
               WHERE project = ?1 AND status = ?2
               ORDER BY updated_at_epoch DESC";
        filter_val = Some(status.to_string());
    } else {
        sql = "SELECT id, project, title, description, status, progress, next_action, blockers,
                      created_at_epoch, updated_at_epoch, completed_at_epoch
               FROM workstreams
               WHERE project = ?1
               ORDER BY updated_at_epoch DESC";
        filter_val = None;
    }

    let mut stmt = conn.prepare(sql)?;
    let rows = if let Some(ref sv) = filter_val {
        stmt.query_map(params![project, sv], map_workstream_row)?
    } else {
        stmt.query_map(params![project], map_workstream_row)?
    };
    crate::db_query::collect_rows(rows)
}

fn map_workstream_row(row: &rusqlite::Row) -> rusqlite::Result<WorkStream> {
    let status_str: String = row.get(4)?;
    Ok(WorkStream {
        id: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: WorkStreamStatus::from_db(&status_str),
        progress: row.get(5)?,
        next_action: row.get(6)?,
        blockers: row.get(7)?,
        created_at_epoch: row.get(8)?,
        updated_at_epoch: row.get(9)?,
        completed_at_epoch: row.get(10)?,
    })
}

pub fn find_matching_workstream(
    conn: &Connection,
    project: &str,
    title: &str,
) -> Result<Option<WorkStream>> {
    // Exact match first
    let exact: Option<WorkStream> = conn
        .query_row(
            "SELECT id, project, title, description, status, progress, next_action, blockers,
                    created_at_epoch, updated_at_epoch, completed_at_epoch
             FROM workstreams
             WHERE project = ?1 AND title = ?2 AND status IN ('active', 'paused')",
            params![project, title],
            map_workstream_row,
        )
        .ok();
    if exact.is_some() {
        return Ok(exact);
    }

    // Fuzzy: title contains or is contained
    let title_lower = title.to_lowercase();
    let mut stmt = conn.prepare(
        "SELECT id, project, title, description, status, progress, next_action, blockers,
                created_at_epoch, updated_at_epoch, completed_at_epoch
         FROM workstreams
         WHERE project = ?1 AND status IN ('active', 'paused')
         ORDER BY updated_at_epoch DESC",
    )?;
    let rows = stmt.query_map(params![project], map_workstream_row)?;
    for row in rows {
        let ws = row?;
        let ws_title_lower = ws.title.to_lowercase();
        if ws_title_lower.contains(&title_lower) || title_lower.contains(&ws_title_lower) {
            return Ok(Some(ws));
        }
    }
    Ok(None)
}

pub fn upsert_workstream(
    conn: &Connection,
    project: &str,
    memory_session_id: &str,
    parsed: &ParsedWorkStream,
) -> Result<i64> {
    let Some(title) = parsed.title.as_deref() else {
        anyhow::bail!("workstream title is required");
    };

    let now = chrono::Utc::now().timestamp();
    let status = if parsed.is_completed {
        WorkStreamStatus::Completed
    } else {
        WorkStreamStatus::Active
    };
    let completed_at = if parsed.is_completed { Some(now) } else { None };

    let ws_id = if let Some(existing) = find_matching_workstream(conn, project, title)? {
        conn.execute(
            "UPDATE workstreams
             SET title = ?1, status = ?2, progress = ?3, next_action = ?4, blockers = ?5,
                 updated_at_epoch = ?6, completed_at_epoch = COALESCE(?7, completed_at_epoch)
             WHERE id = ?8",
            params![
                title,
                status.as_str(),
                parsed.progress,
                parsed.next_action,
                parsed.blockers,
                now,
                completed_at,
                existing.id,
            ],
        )?;
        existing.id
    } else {
        conn.execute(
            "INSERT INTO workstreams
             (project, title, status, progress, next_action, blockers,
              created_at_epoch, updated_at_epoch, completed_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
            params![
                project,
                title,
                status.as_str(),
                parsed.progress,
                parsed.next_action,
                parsed.blockers,
                now,
                completed_at,
            ],
        )?;
        conn.last_insert_rowid()
    };

    // Link session
    conn.execute(
        "INSERT OR IGNORE INTO workstream_sessions (workstream_id, memory_session_id, linked_at_epoch)
         VALUES (?1, ?2, ?3)",
        params![ws_id, memory_session_id, now],
    )?;

    Ok(ws_id)
}

pub fn update_workstream_manual(
    conn: &Connection,
    id: i64,
    status: Option<&str>,
    next_action: Option<&str>,
    blockers: Option<&str>,
) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let mut sets = vec!["updated_at_epoch = ?1".to_string()];
    let mut param_idx = 2u32;

    // Build dynamic SET clause
    let status_val = status.map(|s| WorkStreamStatus::from_db(s));
    if status_val.is_some() {
        sets.push(format!("status = ?{}", param_idx));
        param_idx += 1;
    }
    if next_action.is_some() {
        sets.push(format!("next_action = ?{}", param_idx));
        param_idx += 1;
    }
    if blockers.is_some() {
        sets.push(format!("blockers = ?{}", param_idx));
        param_idx += 1;
    }
    if let Some(sv) = &status_val {
        if *sv == WorkStreamStatus::Completed {
            sets.push(format!("completed_at_epoch = ?{}", param_idx));
            param_idx += 1;
        }
    }

    let sql = format!(
        "UPDATE workstreams SET {} WHERE id = ?{}",
        sets.join(", "),
        param_idx
    );

    // Use a simpler approach: always include all params positionally
    let completed_at = status_val
        .filter(|s| *s == WorkStreamStatus::Completed)
        .map(|_| now);

    let mut dynamic_params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
    if let Some(sv) = &status_val {
        dynamic_params.push(Box::new(sv.as_str().to_string()));
    }
    if let Some(na) = next_action {
        dynamic_params.push(Box::new(na.to_string()));
    }
    if let Some(bl) = blockers {
        dynamic_params.push(Box::new(bl.to_string()));
    }
    if let Some(ca) = completed_at {
        dynamic_params.push(Box::new(ca));
    }
    dynamic_params.push(Box::new(id));

    let refs = crate::db::to_sql_refs(&dynamic_params);
    let affected = conn.execute(&sql, refs.as_slice())?;
    Ok(affected > 0)
}

pub fn auto_pause_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'paused', updated_at_epoch = ?1
         WHERE project = ?2 AND status = 'active' AND updated_at_epoch < ?3",
        params![chrono::Utc::now().timestamp(), project, cutoff],
    )?;
    Ok(count)
}

pub fn auto_abandon_inactive(conn: &Connection, project: &str, days: i64) -> Result<usize> {
    let cutoff = chrono::Utc::now().timestamp() - (days * 86400);
    let count = conn.execute(
        "UPDATE workstreams SET status = 'abandoned', updated_at_epoch = ?1
         WHERE project = ?2 AND status = 'paused' AND updated_at_epoch < ?3",
        params![chrono::Utc::now().timestamp(), project, cutoff],
    )?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_workstream_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE workstreams (
                id INTEGER PRIMARY KEY,
                project TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                progress TEXT,
                next_action TEXT,
                blockers TEXT,
                created_at_epoch INTEGER NOT NULL,
                updated_at_epoch INTEGER NOT NULL,
                completed_at_epoch INTEGER
            );
            CREATE TABLE workstream_sessions (
                id INTEGER PRIMARY KEY,
                workstream_id INTEGER NOT NULL,
                memory_session_id TEXT NOT NULL,
                linked_at_epoch INTEGER NOT NULL,
                UNIQUE(workstream_id, memory_session_id)
            );",
        )
        .unwrap();
    }

    #[test]
    fn test_upsert_creates_new() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let parsed = ParsedWorkStream {
            title: Some("Implement WorkStream".to_string()),
            progress: Some("Started design".to_string()),
            next_action: Some("Write code".to_string()),
            blockers: None,
            is_completed: false,
        };
        let id = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();
        assert!(id > 0);

        let ws = query_active_workstreams(&conn, "test/proj").unwrap();
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].title, "Implement WorkStream");
        assert_eq!(ws[0].status, WorkStreamStatus::Active);
    }

    #[test]
    fn test_upsert_updates_existing() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let parsed1 = ParsedWorkStream {
            title: Some("Feature X".to_string()),
            progress: Some("Step 1 done".to_string()),
            next_action: Some("Step 2".to_string()),
            blockers: None,
            is_completed: false,
        };
        let id1 = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed1).unwrap();

        let parsed2 = ParsedWorkStream {
            title: Some("Feature X".to_string()),
            progress: Some("Step 2 done".to_string()),
            next_action: Some("Step 3".to_string()),
            blockers: None,
            is_completed: false,
        };
        let id2 = upsert_workstream(&conn, "test/proj", "mem-def", &parsed2).unwrap();
        assert_eq!(id1, id2);

        let ws = query_active_workstreams(&conn, "test/proj").unwrap();
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].progress.as_deref(), Some("Step 2 done"));
    }

    #[test]
    fn test_fuzzy_match() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let parsed1 = ParsedWorkStream {
            title: Some("WorkStream 层实现".to_string()),
            progress: Some("设计完成".to_string()),
            next_action: None,
            blockers: None,
            is_completed: false,
        };
        upsert_workstream(&conn, "test/proj", "mem-abc", &parsed1).unwrap();

        // Fuzzy match: substring
        let found = find_matching_workstream(&conn, "test/proj", "WorkStream").unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().title, "WorkStream 层实现");
    }

    #[test]
    fn test_no_match_creates_new() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let parsed1 = ParsedWorkStream {
            title: Some("Feature A".to_string()),
            progress: None,
            next_action: None,
            blockers: None,
            is_completed: false,
        };
        upsert_workstream(&conn, "test/proj", "mem-abc", &parsed1).unwrap();

        let parsed2 = ParsedWorkStream {
            title: Some("Feature B".to_string()),
            progress: None,
            next_action: None,
            blockers: None,
            is_completed: false,
        };
        upsert_workstream(&conn, "test/proj", "mem-def", &parsed2).unwrap();

        let ws = query_active_workstreams(&conn, "test/proj").unwrap();
        assert_eq!(ws.len(), 2);
    }

    #[test]
    fn test_skip_when_title_none() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let parsed = ParsedWorkStream {
            title: None,
            progress: None,
            next_action: None,
            blockers: None,
            is_completed: false,
        };
        let result = upsert_workstream(&conn, "test/proj", "mem-abc", &parsed);
        assert!(result.is_err());
    }

    #[test]
    fn test_completed_status() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let parsed = ParsedWorkStream {
            title: Some("Done Task".to_string()),
            progress: Some("All done".to_string()),
            next_action: None,
            blockers: None,
            is_completed: true,
        };
        upsert_workstream(&conn, "test/proj", "mem-abc", &parsed).unwrap();

        // Should NOT appear in active query
        let active = query_active_workstreams(&conn, "test/proj").unwrap();
        assert_eq!(active.len(), 0);

        // Should appear with completed filter
        let completed = query_workstreams(&conn, "test/proj", Some("completed")).unwrap();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].status, WorkStreamStatus::Completed);
        assert!(completed[0].completed_at_epoch.is_some());
    }

    #[test]
    fn test_only_matches_active_or_paused() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let now = chrono::Utc::now().timestamp();
        conn.execute(
            "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
             VALUES ('test/proj', 'Old Task', 'completed', ?1, ?1)",
            params![now],
        )
        .unwrap();

        let found = find_matching_workstream(&conn, "test/proj", "Old Task").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_auto_pause_after_7_days() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let old_epoch = chrono::Utc::now().timestamp() - (8 * 86400);
        conn.execute(
            "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
             VALUES ('test/proj', 'Stale Task', 'active', ?1, ?1)",
            params![old_epoch],
        )
        .unwrap();

        let paused = auto_pause_inactive(&conn, "test/proj", 7).unwrap();
        assert_eq!(paused, 1);

        let ws = query_workstreams(&conn, "test/proj", Some("paused")).unwrap();
        assert_eq!(ws.len(), 1);
    }

    #[test]
    fn test_auto_abandon_after_30_days() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let old_epoch = chrono::Utc::now().timestamp() - (31 * 86400);
        conn.execute(
            "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
             VALUES ('test/proj', 'Very Stale', 'paused', ?1, ?1)",
            params![old_epoch],
        )
        .unwrap();

        let abandoned = auto_abandon_inactive(&conn, "test/proj", 30).unwrap();
        assert_eq!(abandoned, 1);

        let ws = query_workstreams(&conn, "test/proj", Some("abandoned")).unwrap();
        assert_eq!(ws.len(), 1);
    }

    #[test]
    fn test_auto_pause_skips_recent() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let recent = chrono::Utc::now().timestamp() - (3 * 86400);
        conn.execute(
            "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
             VALUES ('test/proj', 'Recent Task', 'active', ?1, ?1)",
            params![recent],
        )
        .unwrap();

        let paused = auto_pause_inactive(&conn, "test/proj", 7).unwrap();
        assert_eq!(paused, 0);
    }

    #[test]
    fn test_auto_abandon_skips_active() {
        let conn = Connection::open_in_memory().unwrap();
        setup_workstream_schema(&conn);

        let old_epoch = chrono::Utc::now().timestamp() - (31 * 86400);
        conn.execute(
            "INSERT INTO workstreams (project, title, status, created_at_epoch, updated_at_epoch)
             VALUES ('test/proj', 'Old Active', 'active', ?1, ?1)",
            params![old_epoch],
        )
        .unwrap();

        // auto_abandon only targets paused, not active
        let abandoned = auto_abandon_inactive(&conn, "test/proj", 30).unwrap();
        assert_eq!(abandoned, 0);
    }
}
