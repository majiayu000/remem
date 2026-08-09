//! Reference-integrity queries used by the focused CurrentTruth diagnostic.

use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{params, Connection};

use super::{ReferenceIssue, TruthDoctorOptions};

pub(super) fn collect_unreconstructable_historical_refs(
    _conn: &Connection,
    opts: &TruthDoctorOptions,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    if opts.as_of_epoch.is_none() {
        return Ok(());
    }
    out.insert(ReferenceIssue {
        relation_ref: format!("historical_cutoff:{reference_epoch}"),
        claim_ref: format!("truth_scope:{}", opts.project),
        problem: "unreconstructable_historical_truth",
        stored_status: None,
    });
    Ok(())
}

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

pub(super) fn collect_self_referential_memory_edge_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT 'memory_edge:' || me.id, 'memory:' || memory.id, memory.status
         FROM memory_edges me
         JOIN memories memory ON memory.id = me.from_memory_id
         WHERE me.from_memory_id = me.to_memory_id
           AND memory.project = ?1
           AND (?2 IS NULL OR memory.branch IS NULL OR memory.branch = ?2)
           AND (?4 IS NULL OR memory.topic_key = ?4)
           AND me.created_at_epoch <= ?3",
    )?;
    let rows = stmt.query_map(
        params![opts.project, opts.branch, reference_epoch, opts.subject],
        |row| {
            Ok(ReferenceIssue {
                relation_ref: row.get(0)?,
                claim_ref: row.get(1)?,
                problem: "self_referential_relation",
                stored_status: Some(row.get(2)?),
            })
        },
    )?;
    for row in rows {
        out.insert(row?);
    }
    Ok(())
}

pub(super) fn collect_invalid_user_claim_replacement_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT 'user_claim_supersedes:' || current.id,
                'user_claim:' || current.supersedes_claim_id,
                CASE WHEN old.id IS NULL THEN 'dangling_claim_reference'
                     WHEN current.supersedes_claim_id = current.id
                     THEN 'self_referential_relation'
                     ELSE 'replacement_scope_mismatch' END,
                CASE WHEN old.id IS NULL THEN NULL ELSE old.status END
         FROM user_context_claims current
         LEFT JOIN user_context_claims old ON old.id = current.supersedes_claim_id
         WHERE current.owner_scope = 'repo' AND current.owner_key = ?1
           AND current.supersedes_claim_id IS NOT NULL
           AND (old.id IS NULL
             OR current.supersedes_claim_id = current.id
             OR old.owner_scope != current.owner_scope
             OR old.owner_key != current.owner_key)
           AND current.created_at_epoch <= ?2
           AND (?3 IS NULL
                OR current.claim_key = ?3
                OR current.claim_type || ':' || current.claim_key = ?3)",
    )?;
    let rows = stmt.query_map(
        params![opts.project, reference_epoch, opts.subject],
        |row| {
            let problem = match row.get::<_, String>(2)?.as_str() {
                "dangling_claim_reference" => "dangling_claim_reference",
                "self_referential_relation" => "self_referential_relation",
                _ => "replacement_scope_mismatch",
            };
            Ok(ReferenceIssue {
                relation_ref: row.get(0)?,
                claim_ref: row.get(1)?,
                problem,
                stored_status: row.get(3)?,
            })
        },
    )?;
    for row in rows {
        out.insert(row?);
    }
    Ok(())
}
