//! Reference-integrity queries used by the focused CurrentTruth diagnostic.

use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{params, Connection};

use super::{ReferenceIssue, TruthDoctorOptions};

pub(super) fn collect_dangling_memory_edge_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT 'memory_edge:' || me.id,
                CASE WHEN me.from_memory_id IS NULL
                     THEN 'memory_edge:' || me.id || ':from_memory_id:null'
                     ELSE 'memory:' || me.from_memory_id END
         FROM memory_edges me
         LEFT JOIN memories fm ON fm.id = me.from_memory_id
         JOIN memories tm ON tm.id = me.to_memory_id
         LEFT JOIN memory_candidates mc ON mc.id = me.source_candidate_id
         WHERE (me.from_memory_id IS NULL OR fm.id IS NULL)
           AND NOT (me.edge_type = 'derived_from'
                    AND me.from_memory_id IS NULL
                    AND mc.id IS NOT NULL)
           AND tm.project = ?1
           AND (?2 IS NULL OR tm.branch IS NULL OR tm.branch = ?2)
           AND (?4 IS NULL OR tm.topic_key = ?4)
           AND me.created_at_epoch <= ?3
         UNION ALL
         SELECT 'memory_edge:' || me.id,
                CASE WHEN me.to_memory_id IS NULL
                     THEN 'memory_edge:' || me.id || ':to_memory_id:null'
                     ELSE 'memory:' || me.to_memory_id END
         FROM memory_edges me
         JOIN memories fm ON fm.id = me.from_memory_id
         LEFT JOIN memories tm ON tm.id = me.to_memory_id
         WHERE (me.to_memory_id IS NULL OR tm.id IS NULL)
           AND fm.project = ?1
           AND (?2 IS NULL OR fm.branch IS NULL OR fm.branch = ?2)
           AND (?4 IS NULL OR fm.topic_key = ?4)
           AND me.created_at_epoch <= ?3",
    )?;
    let rows = stmt.query_map(
        params![opts.project, opts.branch, reference_epoch, opts.subject],
        |row| {
            Ok(ReferenceIssue {
                relation_ref: row.get(0)?,
                claim_ref: row.get(1)?,
                problem: "dangling_claim_reference",
                stored_status: None,
            })
        },
    )?;
    for row in rows {
        out.insert(row?);
    }
    Ok(())
}
