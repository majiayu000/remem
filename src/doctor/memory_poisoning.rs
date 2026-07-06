use rusqlite::{Connection, OptionalExtension};

use super::database::table_count;
use super::types::{Check, Status};

pub(super) fn check_memory_poisoning_defense(conn: Option<&Connection>) -> Check {
    let Some(conn) = conn else {
        return Check::new(
            "Memory poisoning defense",
            Status::Warn,
            "cannot open database",
        );
    };

    let quarantined = match quarantined_candidate_count(conn) {
        Ok(count) => count,
        Err(err) => {
            return Check::new(
                "Memory poisoning defense",
                Status::Warn,
                format!("cannot load poisoning quarantine stats: {err}"),
            );
        }
    };
    let drop_count = match table_count(conn, "memory_poisoning_injection_drops") {
        Ok(count) => count,
        Err(err) => {
            return Check::new(
                "Memory poisoning defense",
                Status::Warn,
                format!("cannot load poisoning injection drop stats: {err}"),
            );
        }
    };

    let mut detail = format!(
        "pattern_set_version={}, quarantined={}, injection_drops={}",
        crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
        quarantined,
        drop_count
    );
    match top_quarantine_patterns(conn) {
        Ok(patterns) if !patterns.is_empty() => {
            detail.push_str(&format!("; patterns={}", patterns.join(", ")));
        }
        Err(err) => {
            detail.push_str(&format!("; cannot load pattern breakdown: {err}"));
        }
        _ => {}
    }
    if drop_count > 0 {
        match latest_poisoning_drop(conn) {
            Ok(Some(drop)) => {
                detail.push_str(&format!(
                    "; latest_drop=memory:{} pattern={}@v{} title={}",
                    drop.memory_id,
                    drop.pattern_id,
                    drop.pattern_version,
                    crate::db::truncate_str(&drop.title.unwrap_or_default(), 80)
                ));
            }
            Ok(None) => {}
            Err(err) => {
                detail.push_str(&format!("; cannot load latest drop: {err}"));
            }
        }
    }

    if quarantined > 0 || drop_count > 0 {
        Check::new("Memory poisoning defense", Status::Warn, detail)
    } else {
        Check::new("Memory poisoning defense", Status::Ok, detail)
    }
}

fn quarantined_candidate_count(conn: &Connection) -> Result<i64, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'quarantined'",
        [],
        |row| row.get(0),
    )
}

fn top_quarantine_patterns(conn: &Connection) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT COALESCE(quarantine_pattern_id, '<missing>'), COUNT(*)
         FROM memory_candidates
         WHERE review_status = 'quarantined'
         GROUP BY COALESCE(quarantine_pattern_id, '<missing>')
         ORDER BY COUNT(*) DESC, COALESCE(quarantine_pattern_id, '<missing>') ASC
         LIMIT 3",
    )?;
    let rows = stmt.query_map([], |row| {
        let pattern: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        Ok(format!("{pattern}:{count}"))
    })?;
    let mut patterns = Vec::new();
    for row in rows {
        patterns.push(row?);
    }
    Ok(patterns)
}

struct LatestPoisoningDrop {
    memory_id: i64,
    pattern_id: String,
    pattern_version: i64,
    title: Option<String>,
}

fn latest_poisoning_drop(
    conn: &Connection,
) -> Result<Option<LatestPoisoningDrop>, rusqlite::Error> {
    conn.query_row(
        "SELECT memory_id, pattern_id, pattern_version, title
         FROM memory_poisoning_injection_drops
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT 1",
        [],
        |row| {
            Ok(LatestPoisoningDrop {
                memory_id: row.get(0)?,
                pattern_id: row.get(1)?,
                pattern_version: row.get(2)?,
                title: row.get(3)?,
            })
        },
    )
    .optional()
}
