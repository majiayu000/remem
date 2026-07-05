use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

const RESOLVED_STATUSES_SQL: &str = "('approved', 'edited', 'discarded', 'noop')";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewQueueStats {
    pub pending_total: i64,
    pub pending_median_age_secs: Option<i64>,
    pub pending_max_age_secs: Option<i64>,
    pub inflow_7d: i64,
    pub resolved_7d: i64,
    pub projects: Vec<ReviewQueueProjectStats>,
    pub block_reasons: Vec<ReviewQueueBlockReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewQueueProjectStats {
    pub project: Option<String>,
    pub pending: i64,
    pub median_age_secs: Option<i64>,
    pub max_age_secs: Option<i64>,
    pub inflow_7d: i64,
    pub resolved_7d: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReviewQueueBlockReason {
    pub reason: Option<String>,
    pub pending: i64,
    pub example_ids: Vec<i64>,
}

pub(crate) fn query_review_queue_stats(
    conn: &Connection,
    now_epoch: i64,
) -> Result<ReviewQueueStats> {
    let week_ago = now_epoch - 7 * 24 * 3600;

    let (pending_total, pending_max_age_secs) = conn.query_row(
        "SELECT COUNT(*), MAX(?1 - created_at_epoch)
         FROM memory_candidates WHERE review_status = 'pending_review'",
        params![now_epoch],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)),
    )?;
    let pending_median_age_secs = median_pending_age(conn, now_epoch, None, pending_total)?;

    let inflow_7d: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_candidates WHERE created_at_epoch >= ?1",
        params![week_ago],
        |row| row.get(0),
    )?;
    // Legacy rows reviewed before the v055 metadata columns existed have no
    // reviewed_at_epoch; updated_at_epoch is the closest review-time signal.
    let resolved_7d: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM memory_candidates
             WHERE review_status IN {RESOLVED_STATUSES_SQL}
               AND COALESCE(reviewed_at_epoch, updated_at_epoch) >= ?1"
        ),
        params![week_ago],
        |row| row.get(0),
    )?;

    let projects = query_project_stats(conn, now_epoch, week_ago)?;
    let block_reasons = query_block_reasons(conn, None)?;

    Ok(ReviewQueueStats {
        pending_total,
        pending_median_age_secs,
        pending_max_age_secs,
        inflow_7d,
        resolved_7d,
        projects,
        block_reasons,
    })
}

fn query_project_stats(
    conn: &Connection,
    now_epoch: i64,
    week_ago: i64,
) -> Result<Vec<ReviewQueueProjectStats>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT p.project_path,
                SUM(CASE WHEN c.review_status = 'pending_review' THEN 1 ELSE 0 END) AS pending,
                MAX(CASE WHEN c.review_status = 'pending_review'
                         THEN ?1 - c.created_at_epoch END) AS max_age,
                SUM(CASE WHEN c.created_at_epoch >= ?2 THEN 1 ELSE 0 END) AS inflow,
                SUM(CASE WHEN c.review_status IN {RESOLVED_STATUSES_SQL}
                          AND COALESCE(c.reviewed_at_epoch, c.updated_at_epoch) >= ?2
                         THEN 1 ELSE 0 END) AS resolved
         FROM memory_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         GROUP BY p.project_path
         HAVING pending > 0 OR inflow > 0 OR resolved > 0
         ORDER BY pending DESC, p.project_path ASC"
    ))?;
    let rows = stmt
        .query_map(params![now_epoch, week_ago], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    rows.into_iter()
        .map(|(project, pending, max_age_secs, inflow_7d, resolved_7d)| {
            let median_age_secs = median_pending_age(conn, now_epoch, project.as_deref(), pending)?;
            Ok(ReviewQueueProjectStats {
                project,
                pending,
                median_age_secs,
                max_age_secs,
                inflow_7d,
                resolved_7d,
            })
        })
        .collect()
}

fn median_pending_age(
    conn: &Connection,
    now_epoch: i64,
    project: Option<&str>,
    pending_count: i64,
) -> Result<Option<i64>> {
    if pending_count == 0 {
        return Ok(None);
    }
    let offset = (pending_count - 1) / 2;
    let created: Option<i64> = if let Some(project) = project {
        conn.query_row(
            "SELECT c.created_at_epoch
             FROM memory_candidates c
             JOIN projects p ON p.id = c.project_id
             WHERE c.review_status = 'pending_review' AND p.project_path = ?1
             ORDER BY c.created_at_epoch DESC
             LIMIT 1 OFFSET ?2",
            params![project, offset],
            |row| row.get(0),
        )
        .optional()?
    } else {
        conn.query_row(
            "SELECT created_at_epoch FROM memory_candidates
             WHERE review_status = 'pending_review'
             ORDER BY created_at_epoch DESC
             LIMIT 1 OFFSET ?1",
            params![offset],
            |row| row.get(0),
        )
        .optional()?
    };
    Ok(created.map(|epoch| now_epoch.saturating_sub(epoch)))
}

pub(crate) fn query_block_reasons(
    conn: &Connection,
    project: Option<&str>,
) -> Result<Vec<ReviewQueueBlockReason>> {
    let mut stmt = conn.prepare(
        "SELECT c.auto_promote_block_reason, COUNT(*) AS pending
         FROM memory_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE c.review_status = 'pending_review'
           AND (?1 IS NULL OR p.project_path = ?1)
         GROUP BY c.auto_promote_block_reason
         ORDER BY pending DESC, c.auto_promote_block_reason ASC",
    )?;
    let reasons = stmt
        .query_map(params![project], |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    reasons
        .into_iter()
        .map(|(reason, pending)| {
            let example_ids = block_reason_examples(conn, reason.as_deref(), project)?;
            Ok(ReviewQueueBlockReason {
                reason,
                pending,
                example_ids,
            })
        })
        .collect()
}

fn block_reason_examples(
    conn: &Connection,
    reason: Option<&str>,
    project: Option<&str>,
) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT c.id FROM memory_candidates c
         LEFT JOIN projects p ON p.id = c.project_id
         WHERE c.review_status = 'pending_review'
           AND ((?1 IS NULL AND c.auto_promote_block_reason IS NULL)
                OR c.auto_promote_block_reason = ?1)
           AND (?2 IS NULL OR p.project_path = ?2)
         ORDER BY c.created_at_epoch ASC, c.id ASC
         LIMIT 3",
    )?;
    let ids = stmt
        .query_map(params![reason, project], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        crate::migrate::run_migrations(&conn).expect("migrations should run");
        conn
    }

    fn project_id(conn: &Connection, path: &str) -> i64 {
        if let Ok(id) = conn.query_row(
            "SELECT id FROM projects WHERE project_path = ?1",
            params![path],
            |row| row.get(0),
        ) {
            return id;
        }
        conn.execute(
            "INSERT OR IGNORE INTO workspaces
             (root_path, created_at_epoch, updated_at_epoch) VALUES (?1, 0, 0)",
            params![path],
        )
        .expect("workspace insert");
        let workspace_id: i64 = conn
            .query_row(
                "SELECT id FROM workspaces WHERE root_path = ?1",
                params![path],
                |row| row.get(0),
            )
            .expect("workspace id");
        conn.execute(
            "INSERT INTO projects
             (workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?2, ?2, 0, 0)",
            params![workspace_id, path],
        )
        .expect("project insert");
        conn.last_insert_rowid()
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_candidate(
        conn: &Connection,
        project: &str,
        review_status: &str,
        block_reason: Option<&str>,
        created_at: i64,
        reviewed_at: Option<i64>,
        updated_at: i64,
    ) -> i64 {
        let pid = project_id(conn, project);
        conn.execute(
            "INSERT INTO memory_candidates
             (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
              confidence, risk_class, review_status, auto_promote_block_reason,
              created_at_epoch, updated_at_epoch, reviewed_at_epoch)
             VALUES (?1, 'project', 'decision', 'topic', 'text', '[]',
                     0.7, 'medium', ?2, ?3, ?4, ?5, ?6)",
            params![
                pid,
                review_status,
                block_reason,
                created_at,
                updated_at,
                reviewed_at
            ],
        )
        .expect("candidate insert");
        conn.last_insert_rowid()
    }

    #[test]
    fn review_queue_stats_aggregates_pending_ages() -> Result<()> {
        let conn = setup_conn();
        let now = 1_000_000;
        insert_candidate(&conn, "/p/a", "pending_review", None, now - 100, None, now);
        insert_candidate(&conn, "/p/a", "pending_review", None, now - 500, None, now);
        insert_candidate(&conn, "/p/a", "pending_review", None, now - 900, None, now);

        let stats = query_review_queue_stats(&conn, now)?;

        assert_eq!(stats.pending_total, 3);
        assert_eq!(stats.pending_median_age_secs, Some(500));
        assert_eq!(stats.pending_max_age_secs, Some(900));
        assert_eq!(stats.projects.len(), 1);
        assert_eq!(stats.projects[0].pending, 3);
        assert_eq!(stats.projects[0].median_age_secs, Some(500));
        Ok(())
    }

    #[test]
    fn review_queue_stats_counts_inflow_and_resolved_by_review_time() -> Result<()> {
        let conn = setup_conn();
        let now = 100 * 24 * 3600;
        let old = now - 30 * 24 * 3600;
        let recent = now - 3600;
        // Created long ago but resolved inside the window: counts as resolved.
        insert_candidate(&conn, "/p/a", "discarded", None, old, Some(recent), recent);
        // Legacy row without reviewed_at_epoch falls back to updated_at_epoch.
        insert_candidate(&conn, "/p/a", "approved", None, old, None, recent);
        // Resolved outside the window: does not count.
        insert_candidate(&conn, "/p/a", "noop", None, old, Some(old), old);
        // Created inside the window: counts as inflow.
        insert_candidate(&conn, "/p/a", "pending_review", None, recent, None, recent);

        let stats = query_review_queue_stats(&conn, now)?;

        assert_eq!(stats.inflow_7d, 1);
        assert_eq!(stats.resolved_7d, 2);
        assert_eq!(stats.projects.len(), 1);
        assert_eq!(stats.projects[0].inflow_7d, 1);
        assert_eq!(stats.projects[0].resolved_7d, 2);
        Ok(())
    }

    #[test]
    fn review_queue_stats_reports_block_reasons_with_examples() -> Result<()> {
        let conn = setup_conn();
        let now = 1_000_000;
        let a = insert_candidate(
            &conn,
            "/p/a",
            "pending_review",
            Some("risk_class_not_low"),
            now - 300,
            None,
            now,
        );
        let b = insert_candidate(
            &conn,
            "/p/a",
            "pending_review",
            Some("risk_class_not_low"),
            now - 200,
            None,
            now,
        );
        insert_candidate(&conn, "/p/a", "pending_review", None, now - 100, None, now);

        let stats = query_review_queue_stats(&conn, now)?;

        assert_eq!(stats.block_reasons.len(), 2);
        assert_eq!(
            stats.block_reasons[0].reason.as_deref(),
            Some("risk_class_not_low")
        );
        assert_eq!(stats.block_reasons[0].pending, 2);
        assert_eq!(stats.block_reasons[0].example_ids, vec![a, b]);
        assert_eq!(stats.block_reasons[1].reason, None);
        assert_eq!(stats.block_reasons[1].pending, 1);
        Ok(())
    }

    #[test]
    fn review_queue_stats_empty_db_yields_zeroes() -> Result<()> {
        let conn = setup_conn();
        let stats = query_review_queue_stats(&conn, 1_000)?;
        assert_eq!(stats.pending_total, 0);
        assert_eq!(stats.pending_median_age_secs, None);
        assert_eq!(stats.pending_max_age_secs, None);
        assert!(stats.projects.is_empty());
        assert!(stats.block_reasons.is_empty());
        Ok(())
    }
}
