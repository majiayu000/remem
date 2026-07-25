use anyhow::Result;
use rusqlite::Connection;

use super::shared::collect_rows;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserContextStats {
    pub claims_total: i64,
    pub claims_active: i64,
    pub claims_suppressed: i64,
    pub claims_deleted: i64,
    pub candidates_total: i64,
    pub candidates_pending_review: i64,
    pub candidates_auto_promoted: i64,
    pub candidate_block_reasons: Vec<UserContextBlockReasonStat>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserContextBlockReasonStat {
    pub reason: Option<String>,
    pub pending: i64,
}

pub fn query_user_context_stats(conn: &Connection) -> Result<UserContextStats> {
    if !crate::retrieval::temporal::sqlite_table_exists(conn, "user_context_claims")?
        || !crate::retrieval::temporal::sqlite_table_exists(conn, "user_context_candidates")?
    {
        return Ok(UserContextStats::default());
    }

    let candidate_block_reasons = {
        let mut stmt = conn.prepare(
            "SELECT auto_promote_block_reason, COUNT(*) AS pending
             FROM user_context_candidates
             WHERE review_status = 'pending_review'
             GROUP BY auto_promote_block_reason
             ORDER BY pending DESC, auto_promote_block_reason ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(UserContextBlockReasonStat {
                reason: row.get(0)?,
                pending: row.get(1)?,
            })
        })?;
        collect_rows(rows)?
    };

    Ok(UserContextStats {
        claims_total: count_claims(conn, None)?,
        claims_active: count_claims(conn, Some("active"))?,
        claims_suppressed: count_claims(conn, Some("suppressed"))?,
        claims_deleted: count_claims(conn, Some("deleted"))?,
        candidates_total: count_candidates(conn, None)?,
        candidates_pending_review: count_candidates(conn, Some("pending_review"))?,
        candidates_auto_promoted: count_candidates(conn, Some("auto_promoted"))?,
        candidate_block_reasons,
    })
}

fn count_claims(conn: &Connection, status: Option<&str>) -> Result<i64> {
    match status {
        Some(status) => conn
            .query_row(
                "SELECT COUNT(*) FROM user_context_claims WHERE status = ?1",
                [status],
                |row| row.get(0),
            )
            .map_err(Into::into),
        None => conn
            .query_row("SELECT COUNT(*) FROM user_context_claims", [], |row| {
                row.get(0)
            })
            .map_err(Into::into),
    }
}

fn count_candidates(conn: &Connection, review_status: Option<&str>) -> Result<i64> {
    match review_status {
        Some(review_status) => conn
            .query_row(
                "SELECT COUNT(*) FROM user_context_candidates WHERE review_status = ?1",
                [review_status],
                |row| row.get(0),
            )
            .map_err(Into::into),
        None => conn
            .query_row("SELECT COUNT(*) FROM user_context_candidates", [], |row| {
                row.get(0)
            })
            .map_err(Into::into),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    #[test]
    fn user_context_stats_report_claims_candidates_and_block_reasons() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE user_context_claims (
                id INTEGER PRIMARY KEY,
                status TEXT NOT NULL
            );
            CREATE TABLE user_context_candidates (
                id INTEGER PRIMARY KEY,
                review_status TEXT NOT NULL,
                auto_promote_block_reason TEXT
            );
            INSERT INTO user_context_claims (status) VALUES
                ('active'),
                ('active'),
                ('suppressed'),
                ('deleted'),
                ('superseded');
            INSERT INTO user_context_candidates (review_status, auto_promote_block_reason) VALUES
                ('pending_review', 'source_not_user_authored'),
                ('pending_review', 'source_not_user_authored'),
                ('pending_review', 'low_confidence'),
                ('auto_promoted', NULL),
                ('approved', NULL);",
        )?;

        let stats = query_user_context_stats(&conn)?;

        assert_eq!(
            stats,
            UserContextStats {
                claims_total: 5,
                claims_active: 2,
                claims_suppressed: 1,
                claims_deleted: 1,
                candidates_total: 5,
                candidates_pending_review: 3,
                candidates_auto_promoted: 1,
                candidate_block_reasons: vec![
                    UserContextBlockReasonStat {
                        reason: Some("source_not_user_authored".to_string()),
                        pending: 2,
                    },
                    UserContextBlockReasonStat {
                        reason: Some("low_confidence".to_string()),
                        pending: 1,
                    },
                ],
            }
        );
        Ok(())
    }

    #[test]
    fn user_context_stats_default_when_tables_are_absent() -> Result<()> {
        let conn = Connection::open_in_memory()?;

        let stats = query_user_context_stats(&conn)?;

        assert_eq!(stats, UserContextStats::default());
        Ok(())
    }
}
