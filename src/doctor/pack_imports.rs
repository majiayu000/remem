use rusqlite::Connection;

use super::types::{Check, Status};

pub(super) fn check_pack_imports(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new("Pack imports", Status::Warn, "cannot open database");
    };

    let memory_counts = match pack_memory_counts(conn) {
        Ok(counts) => counts,
        Err(err) => {
            return Check::new(
                "Pack imports",
                Status::Warn,
                format!("cannot load imported pack memory counts: {err}"),
            );
        }
    };
    let candidate_counts = match pack_candidate_counts(conn) {
        Ok(counts) => counts,
        Err(err) => {
            return Check::new(
                "Pack imports",
                Status::Warn,
                format!("cannot load imported pack review counts: {err}"),
            );
        }
    };

    if memory_counts.is_empty() && candidate_counts.is_empty() {
        return Check::new(
            "Pack imports",
            Status::Ok,
            "no imported pack memories or review candidates",
        );
    }

    let mut details = memory_counts
        .into_iter()
        .map(|(origin, count)| format!("{origin} memories={count}"))
        .collect::<Vec<_>>();
    details.extend(
        candidate_counts
            .into_iter()
            .map(|(origin, pending_review, quarantined)| {
                format!(
                    "{origin} candidates pending_review={pending_review} quarantined={quarantined}"
                )
            }),
    );

    Check::new("Pack imports", Status::Ok, details.join("; "))
}

fn pack_memory_counts(conn: &Connection) -> Result<Vec<(String, i64)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(topic_domain, ''), 'pack:<unknown>') AS origin,
                COUNT(*)
         FROM memories
         WHERE source_trust_class = 'pack'
           AND status = 'active'
         GROUP BY origin
         ORDER BY origin",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

fn pack_candidate_counts(conn: &Connection) -> Result<Vec<(String, i64, i64)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(NULLIF(topic_domain, ''), 'pack:<unknown>') AS origin,
                SUM(CASE WHEN review_status = 'pending_review' THEN 1 ELSE 0 END),
                SUM(CASE WHEN review_status = 'quarantined' THEN 1 ELSE 0 END)
         FROM memory_candidates
         WHERE source_kind = 'pack'
           AND review_status IN ('pending_review', 'quarantined')
         GROUP BY origin
         ORDER BY origin",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect()
}
