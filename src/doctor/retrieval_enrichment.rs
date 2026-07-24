//! `Retrieval enrichment coverage` doctor check (GH-850).
//!
//! Reports the compatibility floor/epoch/state plus eligible / ready /
//! pending / failed counts, recomputes the source identity for ready rows
//! (drift detection), and cross-checks vector consistency when the embedding
//! provider is enabled. Database or hash errors fail closed instead of being
//! folded into a healthy-looking 0.

use rusqlite::Connection;

use super::types::{Check, Status};
use crate::memory::retrieval_enrichment::{
    compatibility_state, enrichment_source_hash, RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION,
    RETRIEVAL_ENRICHMENT_VERSION,
};

const CHECK_NAME: &str = "Retrieval enrichment coverage";

pub(super) fn check_retrieval_enrichment(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new(CHECK_NAME, Status::Fail, "database unavailable");
    };
    match evaluate(conn) {
        Ok(check) => check,
        Err(error) => Check::new(
            CHECK_NAME,
            Status::Fail,
            format!("coverage query failed: {error}"),
        ),
    }
}

struct CoverageCounts {
    eligible: i64,
    ready: i64,
    failed: i64,
    drift: i64,
    vector_consistent: i64,
}

fn evaluate(conn: &Connection) -> anyhow::Result<Check> {
    let Some(state) = compatibility_state(conn)? else {
        return Ok(Check::new(
            CHECK_NAME,
            Status::Fail,
            "retrieval_enrichment_compatibility table is missing; run `remem install` to migrate",
        ));
    };
    let binary = RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION;
    let convergence = format!(
        "floor=v{} target=v{} epoch={} state={} binary=v{binary}",
        state.min_security_policy_version,
        state.target_security_policy_version,
        state.compatibility_epoch,
        state.convergence_state
    );
    if binary < state.min_security_policy_version || state.convergence_state != "ready" {
        return Ok(Check::new(
            CHECK_NAME,
            Status::Fail,
            format!("retrieval is fail-closed until policy convergence completes ({convergence})"),
        ));
    }

    let counts = coverage_counts(conn)?;
    if counts.eligible == 0 {
        return Ok(Check::new(
            CHECK_NAME,
            Status::Ok,
            format!("0/0 eligible memories ({convergence})"),
        ));
    }
    if counts.drift > 0 {
        return Ok(Check::new(
            CHECK_NAME,
            Status::Fail,
            format!(
                "{} ready row(s) have a stale source identity; enrichment must be regenerated \
                 ({convergence})",
                counts.drift
            ),
        ));
    }

    let provider_enabled = !crate::retrieval::embedding::provider_disabled_or_error()?;
    let pending = counts.eligible - counts.ready;
    let vector_note = if provider_enabled {
        format!(
            "vector-consistent={}/{}",
            counts.vector_consistent, counts.ready
        )
    } else {
        "vector=disabled".to_string()
    };
    let detail = format!(
        "{}/{} ready (pending={} failed={} generator=v{RETRIEVAL_ENRICHMENT_VERSION} {vector_note} {convergence})",
        counts.ready, counts.eligible, pending, counts.failed
    );
    let fully_covered = counts.ready == counts.eligible
        && (!provider_enabled || counts.vector_consistent == counts.ready);
    if fully_covered {
        Ok(Check::new(CHECK_NAME, Status::Ok, detail))
    } else {
        Ok(Check::new(
            CHECK_NAME,
            Status::Warn,
            format!(
                "{detail}; run `remem worker --once` (or keep the worker daemon running) and check \
                 error-level enrichment logs for failed rows"
            ),
        ))
    }
}

fn coverage_counts(conn: &Connection) -> anyhow::Result<CoverageCounts> {
    let (eligible, ready, failed, vector_consistent): (i64, i64, i64, i64) = conn.query_row(
        "SELECT COUNT(*),
                SUM(CASE WHEN search_context_enrichment_version = ?1
                          AND search_context_security_policy_version = ?2
                          AND search_context_source_hash IS NOT NULL THEN 1 ELSE 0 END),
                SUM(CASE WHEN search_context_failure_count > 0 THEN 1 ELSE 0 END),
                SUM(CASE WHEN search_context_index_hash IS NOT NULL
                          AND EXISTS (
                              SELECT 1 FROM memory_embeddings e
                              WHERE e.memory_id = memories.id
                                AND e.content_hash = memories.search_context_index_hash
                          ) THEN 1 ELSE 0 END)
         FROM memories
         WHERE status IN ('active', 'stale', 'archived')",
        rusqlite::params![
            RETRIEVAL_ENRICHMENT_VERSION,
            RETRIEVAL_ENRICHMENT_SECURITY_POLICY_VERSION
        ],
        |row| {
            Ok((
                row.get(0)?,
                row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                row.get::<_, Option<i64>>(3)?.unwrap_or(0),
            ))
        },
    )?;

    // Recompute the source identity for ready rows: a ready marker bound to
    // bytes that no longer match is drift, never coverage.
    let mut drift = 0i64;
    let mut stmt = conn.prepare(
        "SELECT title, content, memory_type, topic_key, files, search_context_source_hash
         FROM memories
         WHERE status IN ('active', 'stale', 'archived')
           AND search_context_source_hash IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (title, content, memory_type, topic_key, files, stored) = row?;
        let live = enrichment_source_hash(
            &title,
            &content,
            &memory_type,
            topic_key.as_deref(),
            files.as_deref(),
        );
        if live != stored {
            drift += 1;
        }
    }

    Ok(CoverageCounts {
        eligible,
        ready,
        failed,
        drift,
        vector_consistent,
    })
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;

    fn migrated_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate::run_migrations(&conn).unwrap();
        crate::retrieval::vector::ensure_vec_table(&conn).unwrap();
        conn
    }

    fn insert_row(conn: &Connection, id: i64) {
        conn.execute(
            "INSERT INTO memories (id, project, title, content, memory_type,
                 created_at_epoch, updated_at_epoch, status)
             VALUES (?1, 'proj', 'T', 'body', 'decision', 1, 1, 'active')",
            [id],
        )
        .unwrap();
    }

    #[test]
    fn zero_eligible_reports_ok() {
        let conn = migrated_conn();
        let check = check_retrieval_enrichment(Some(&conn));
        assert!(matches!(check.status, Status::Ok));
        assert!(check.detail.contains("0/0"));
    }

    #[test]
    fn missing_database_fails_closed() {
        let check = check_retrieval_enrichment(None);
        assert!(matches!(check.status, Status::Fail));
    }

    #[test]
    fn partial_coverage_warns_with_recovery_action() {
        let conn = migrated_conn();
        insert_row(&conn, 1);
        insert_row(&conn, 2);
        let check = check_retrieval_enrichment(Some(&conn));
        assert!(matches!(check.status, Status::Warn));
        assert!(check.detail.contains("0/2 ready"));
        assert!(check.detail.contains("remem worker"));
    }

    #[test]
    fn source_identity_drift_fails() {
        let conn = migrated_conn();
        insert_row(&conn, 1);
        // Mark ready with a hash that does not match the live bytes, bypassing
        // the convergence trigger by resetting the fallback in-statement.
        conn.execute(
            "UPDATE memories SET
                 search_context_enrichment_version = 1,
                 search_context_security_policy_version = 1,
                 search_context_source_hash = 'tampered',
                 search_context_fallback_source_hash = 'tampered'
             WHERE id = 1",
            [],
        )
        .unwrap();
        let check = check_retrieval_enrichment(Some(&conn));
        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("stale source identity"));
    }

    #[test]
    fn policy_convergence_in_progress_fails() {
        let conn = migrated_conn();
        conn.execute(
            "UPDATE retrieval_enrichment_compatibility
             SET target_security_policy_version = 2, compatibility_epoch = 2,
                 convergence_state = 'rebuilding' WHERE id = 1",
            [],
        )
        .unwrap();
        let check = check_retrieval_enrichment(Some(&conn));
        assert!(matches!(check.status, Status::Fail));
        assert!(check.detail.contains("fail-closed"));
    }
}
