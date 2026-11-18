//! Reference-integrity queries used by the focused CurrentTruth diagnostic.
//!
//! Memory and graph edge checks cover an edge when either endpoint is one of
//! the memory rows the projection admits (`DoctorScope`), so they follow the
//! projection's alias, owner and target routing (#1136).

use std::collections::BTreeSet;

use anyhow::Result;
use rusqlite::{params, Connection, Params};

use super::scope::DoctorScope;
use super::{ReferenceIssue, TruthDoctorOptions};

pub(super) fn load_reference_issues(
    conn: &Connection,
    opts: &TruthDoctorOptions,
    scope: &DoctorScope,
    reference_epoch: i64,
) -> Result<Vec<ReferenceIssue>> {
    let mut issues = BTreeSet::new();
    collect_unreconstructable_historical_refs(opts, reference_epoch, &mut issues);
    collect_noncurrent_memory_edge_refs(conn, scope, reference_epoch, &mut issues)?;
    collect_dangling_memory_edge_refs(conn, scope, reference_epoch, &mut issues)?;
    collect_self_referential_memory_edge_refs(conn, scope, reference_epoch, &mut issues)?;
    collect_noncurrent_graph_edge_refs(conn, scope, reference_epoch, &mut issues)?;
    collect_dangling_graph_edge_refs(conn, scope, reference_epoch, &mut issues)?;
    collect_invalid_user_claim_replacement_refs(conn, opts, reference_epoch, &mut issues)?;
    collect_noncurrent_user_claim_refs(conn, opts, reference_epoch, &mut issues)?;
    Ok(issues.into_iter().collect())
}

fn collect_unreconstructable_historical_refs(
    opts: &TruthDoctorOptions,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) {
    if opts.as_of_epoch.is_none() {
        return;
    }
    out.insert(ReferenceIssue {
        relation_ref: format!("historical_cutoff:{reference_epoch}"),
        claim_ref: format!("truth_scope:{}", opts.project),
        problem: "unreconstructable_historical_truth",
        stored_status: None,
    });
}

fn collect_noncurrent_memory_edge_refs(
    conn: &Connection,
    scope: &DoctorScope,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    collect_reference_rows(
        conn,
        out,
        "SELECT 'memory_edge:' || me.id, 'memory:' || endpoint.id, endpoint.status
         FROM memory_edges me
         JOIN memories old ON old.id = me.from_memory_id
         JOIN memories new ON new.id = me.to_memory_id
         JOIN memories endpoint ON endpoint.id IN (me.from_memory_id, me.to_memory_id)
         WHERE (endpoint.status != 'active'
             OR endpoint.created_at_epoch > ?2
             OR endpoint.valid_from_epoch > ?2
             OR endpoint.valid_to_epoch <= ?2
             OR endpoint.expires_at_epoch <= ?2)
           AND (old.id IN (SELECT value FROM json_each(?1))
             OR new.id IN (SELECT value FROM json_each(?1)))
           AND me.created_at_epoch <= ?2
           AND NOT (me.edge_type IN ('supersedes', 'merged_into', 'duplicates')
                    AND endpoint.id = me.from_memory_id
                    AND endpoint.created_at_epoch <= ?2
                    AND (endpoint.valid_from_epoch IS NULL OR endpoint.valid_from_epoch <= ?2)
                    AND endpoint.status IN ('active', 'stale', 'superseded', 'archived')
                    AND (endpoint.status != 'active'
                      OR endpoint.valid_to_epoch <= ?2
                      OR endpoint.expires_at_epoch <= ?2))",
        params![scope.memory_ids_json, reference_epoch],
        "references_noncurrent_claim",
    )
}

fn collect_dangling_memory_edge_refs(
    conn: &Connection,
    scope: &DoctorScope,
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
           AND tm.id IN (SELECT value FROM json_each(?1))
           AND me.created_at_epoch <= ?2
         UNION ALL
         SELECT 'memory_edge:' || me.id,
                CASE WHEN me.to_memory_id IS NULL
                     THEN 'memory_edge:' || me.id || ':to_memory_id:null'
                     ELSE 'memory:' || me.to_memory_id END
         FROM memory_edges me
         JOIN memories fm ON fm.id = me.from_memory_id
         LEFT JOIN memories tm ON tm.id = me.to_memory_id
         WHERE (me.to_memory_id IS NULL OR tm.id IS NULL)
           AND fm.id IN (SELECT value FROM json_each(?1))
           AND me.created_at_epoch <= ?2",
    )?;
    let rows = stmt.query_map(params![scope.memory_ids_json, reference_epoch], |row| {
        Ok(ReferenceIssue {
            relation_ref: row.get(0)?,
            claim_ref: row.get(1)?,
            problem: "dangling_claim_reference",
            stored_status: None,
        })
    })?;
    for row in rows {
        out.insert(row?);
    }
    Ok(())
}

fn collect_self_referential_memory_edge_refs(
    conn: &Connection,
    scope: &DoctorScope,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    collect_reference_rows(
        conn,
        out,
        "SELECT 'memory_edge:' || me.id, 'memory:' || memory.id, memory.status
         FROM memory_edges me
         JOIN memories memory ON memory.id = me.from_memory_id
         WHERE me.from_memory_id = me.to_memory_id
           AND memory.id IN (SELECT value FROM json_each(?1))
           AND me.created_at_epoch <= ?2",
        params![scope.memory_ids_json, reference_epoch],
        "self_referential_relation",
    )
}

fn collect_noncurrent_graph_edge_refs(
    conn: &Connection,
    scope: &DoctorScope,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    collect_reference_rows(
        conn,
        out,
        "SELECT 'graph_edge:' || ge.id, 'memory:' || endpoint.id, endpoint.status
         FROM graph_edges ge
         JOIN memories new ON ge.from_node_kind = 'memory' AND new.id = ge.from_node_id
         JOIN memories old ON ge.to_node_kind = 'memory' AND old.id = ge.to_node_id
         JOIN memories endpoint ON endpoint.id IN (ge.from_node_id, ge.to_node_id)
         WHERE ge.edge_trust = 'trusted'
           AND (endpoint.status != 'active'
             OR endpoint.created_at_epoch > ?2
             OR endpoint.valid_from_epoch > ?2
             OR endpoint.valid_to_epoch <= ?2
             OR endpoint.expires_at_epoch <= ?2)
           AND (old.id IN (SELECT value FROM json_each(?1))
             OR new.id IN (SELECT value FROM json_each(?1)))
           AND ge.created_at_epoch <= ?2
           AND (ge.valid_from_epoch IS NULL OR ge.valid_from_epoch <= ?2)
           AND (ge.valid_to_epoch IS NULL OR ge.valid_to_epoch > ?2)
           AND NOT (((ge.edge_type = 'supersedes' AND endpoint.id = ge.to_node_id)
                  OR (ge.edge_type = 'merged_into' AND endpoint.id = ge.from_node_id))
                    AND endpoint.created_at_epoch <= ?2
                    AND (endpoint.valid_from_epoch IS NULL OR endpoint.valid_from_epoch <= ?2)
                    AND endpoint.status IN ('active', 'stale', 'superseded', 'archived')
                    AND (endpoint.status != 'active'
                      OR endpoint.valid_to_epoch <= ?2
                      OR endpoint.expires_at_epoch <= ?2))",
        params![scope.memory_ids_json, reference_epoch],
        "references_noncurrent_claim",
    )
}

fn collect_dangling_graph_edge_refs(
    conn: &Connection,
    scope: &DoctorScope,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT 'graph_edge:' || ge.id,
                CASE WHEN fm.id IS NULL THEN 'memory:' || ge.from_node_id
                     ELSE 'memory:' || ge.to_node_id END
         FROM graph_edges ge
         LEFT JOIN memories fm ON ge.from_node_kind = 'memory' AND fm.id = ge.from_node_id
         LEFT JOIN memories tm ON ge.to_node_kind = 'memory' AND tm.id = ge.to_node_id
         WHERE ge.edge_trust = 'trusted'
           AND ge.from_node_kind = 'memory' AND ge.to_node_kind = 'memory'
           AND ge.created_at_epoch <= ?2
           AND (ge.valid_from_epoch IS NULL OR ge.valid_from_epoch <= ?2)
           AND (ge.valid_to_epoch IS NULL OR ge.valid_to_epoch > ?2)
           AND ((fm.id IS NULL AND tm.id IN (SELECT value FROM json_each(?1)))
             OR (tm.id IS NULL AND fm.id IN (SELECT value FROM json_each(?1))))",
    )?;
    let rows = stmt.query_map(params![scope.memory_ids_json, reference_epoch], |row| {
        Ok(ReferenceIssue {
            relation_ref: row.get(0)?,
            claim_ref: row.get(1)?,
            problem: "dangling_claim_reference",
            stored_status: None,
        })
    })?;
    for row in rows {
        out.insert(row?);
    }
    Ok(())
}

fn collect_invalid_user_claim_replacement_refs(
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

fn collect_noncurrent_user_claim_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
    reference_epoch: i64,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    collect_reference_rows(
        conn,
        out,
        "SELECT 'user_claim_supersedes:' || current.id,
                'user_claim:' || endpoint.id, endpoint.status
         FROM user_context_claims current
         JOIN user_context_claims old ON old.id = current.supersedes_claim_id
         JOIN user_context_claims endpoint ON endpoint.id IN (current.id, old.id)
         WHERE current.owner_scope = 'repo' AND current.owner_key = ?1
           AND old.owner_scope = current.owner_scope AND old.owner_key = current.owner_key
           AND current.created_at_epoch <= ?2
           AND (endpoint.status != 'active'
             OR endpoint.created_at_epoch > ?2
             OR endpoint.valid_from_epoch > ?2
             OR endpoint.valid_to_epoch <= ?2)
           AND NOT (endpoint.id = old.id
                    AND endpoint.created_at_epoch <= ?2
                    AND (endpoint.valid_from_epoch IS NULL OR endpoint.valid_from_epoch <= ?2)
                    AND endpoint.status IN ('active', 'stale', 'superseded')
                    AND (endpoint.status != 'active' OR endpoint.valid_to_epoch <= ?2))
           AND (?3 IS NULL
                OR current.claim_key = ?3
                OR current.claim_type || ':' || current.claim_key = ?3
                OR old.claim_key = ?3
                OR old.claim_type || ':' || old.claim_key = ?3)",
        params![opts.project, reference_epoch, opts.subject],
        "references_noncurrent_claim",
    )
}

fn collect_reference_rows<P: Params>(
    conn: &Connection,
    out: &mut BTreeSet<ReferenceIssue>,
    sql: &str,
    params: P,
    problem: &'static str,
) -> Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok(ReferenceIssue {
            relation_ref: row.get(0)?,
            claim_ref: row.get(1)?,
            problem,
            stored_status: Some(row.get(2)?),
        })
    })?;
    for row in rows {
        out.insert(row?);
    }
    Ok(())
}
