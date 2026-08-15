//! Read-only adapter mapping existing canonical tables into the CurrentTruth
//! read DTOs (GH933 Phase A). SELECT-only: no writes, no migrations.

use std::collections::HashMap;

use anyhow::{Context, Result};
use rusqlite::Connection;

mod context;

use super::lifecycle::{memory_lifecycle, user_claim_lifecycle};
use super::types::{
    ClaimRelationKind, ClaimSource, ClaimView, EvidenceKind, EvidenceTrust, EvidenceView,
    RelationView, TruthQuery, Visibility,
};

pub(crate) fn memory_ref(id: i64) -> String {
    format!("memory:{id}")
}

pub(crate) fn user_claim_ref(id: i64) -> String {
    format!("user_claim:{id}")
}

/// Load memory-backed claims plus relations for one project scope.
///
/// Branch semantics: `Some(branch)` returns branch-neutral rows and rows
/// tagged with exactly that branch; `None` returns all rows (branch-agnostic
/// view). Scope isolation follows the audited canonical-project alias set and
/// repo ownership, so other projects never leak into the result.
pub fn load_memory_claim_groups(
    conn: &Connection,
    query: &TruthQuery,
) -> Result<(Vec<ClaimView>, Vec<RelationView>)> {
    load_memory_claim_groups_at(conn, query, reference_epoch(query))
}

pub(crate) fn load_memory_claim_groups_at(
    conn: &Connection,
    query: &TruthQuery,
    reference_epoch: i64,
) -> Result<(Vec<ClaimView>, Vec<RelationView>)> {
    load_memory_claim_groups_diagnostic(conn, query, reference_epoch)
}

pub(crate) fn load_memory_claim_groups_for_context_at(
    conn: &Connection,
    query: &TruthQuery,
    reference_epoch: i64,
    relevant_memory_ids: &[i64],
) -> Result<(Vec<ClaimView>, Vec<RelationView>)> {
    context::load(conn, query, reference_epoch, relevant_memory_ids)
}

fn load_memory_claim_groups_diagnostic(
    conn: &Connection,
    query: &TruthQuery,
    reference_epoch: i64,
) -> Result<(Vec<ClaimView>, Vec<RelationView>)> {
    let (canonical_scope, projects) = diagnostic_project_scope(conn, &query.project)?;
    let projects_json = serde_json::to_string(&projects)?;
    let sql = "SELECT memories.id, memories.topic_key, memories.title, memories.content,
                memories.project, memories.branch, memories.status,
                memories.valid_from_epoch, memories.valid_to_epoch, memories.expires_at_epoch,
                memories.created_at_epoch, memories.updated_at_epoch,
                memories.evidence_event_ids, memories.source_trust_class,
                memories.memory_type, memories.state_key_id,
                COALESCE(memories.owner_scope, 'repo'),
                COALESCE(memories.owner_key, memories.target_project, memories.project),
                state_key.state_key, memories.owner_scope IS NOT NULL
         FROM memories
         LEFT JOIN memory_state_keys state_key ON state_key.id = memories.state_key_id
         WHERE ((memories.owner_scope = 'repo'
                   AND memories.owner_key IN (SELECT value FROM json_each(?1)))
                OR (memories.owner_scope = 'repo'
                   AND memories.target_project IN (SELECT value FROM json_each(?1)))
                OR (memories.owner_scope IS NULL
                   AND memories.project IN (SELECT value FROM json_each(?1))
                   AND COALESCE(memories.scope, 'project') != 'global'))
           AND (?2 IS NULL OR memories.branch IS NULL OR memories.branch = ?2)
         ORDER BY memories.updated_at_epoch ASC, memories.id ASC";
    let mut stmt = conn.prepare(sql).context("prepare memories truth query")?;
    let rows = stmt
        .query_map(rusqlite::params![projects_json, query.branch], |row| {
            Ok(MemoryRow {
                id: row.get(0)?,
                topic_key: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                _project: row.get(4)?,
                branch: row.get(5)?,
                status: row.get(6)?,
                valid_from_epoch: row.get(7)?,
                valid_to_epoch: row.get(8)?,
                expires_at_epoch: row.get(9)?,
                created_at_epoch: row.get(10)?,
                updated_at_epoch: row.get(11)?,
                evidence_event_ids: row.get(12)?,
                source_trust_class: row.get(13)?,
                memory_type: row.get(14)?,
                owner_scope: row.get(16)?,
                owner_key: row.get(17)?,
                state_key: row.get(18)?,
                owner_explicit: row.get(19)?,
            })
        })
        .context("query memories for truth projection")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read memory truth rows")?;

    let mut claims = Vec::with_capacity(rows.len());
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let visibility = super::visibility::classify_memories(conn, &ids, reference_epoch)?;
    let canonical_owners = canonical_owner_keys(conn, &rows)?;
    for row in rows {
        let row_visibility = visibility
            .get(&row.id)
            .copied()
            .context("missing memory visibility projection row")?;
        let mut claim = memory_claim_view(conn, row, &canonical_scope, &canonical_owners)?;
        if !row_visibility.current_context_eligible {
            claim.lifecycle.visibility = Visibility::Suppressed;
        }
        claims.push(claim);
    }
    let relations = load_memory_relations(conn, &ids, false)?;
    Ok((claims, relations))
}

fn diagnostic_project_scope(conn: &Connection, requested: &str) -> Result<(String, Vec<String>)> {
    match crate::project_alias::resolve_project_identity(conn, requested) {
        Ok(resolution) => {
            let mut projects = resolution.active_aliases;
            projects.push(resolution.canonical_path.clone());
            projects.sort();
            projects.dedup();
            Ok((resolution.canonical_path, projects))
        }
        Err(error) => {
            let exact_count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM projects WHERE project_path = ?1",
                [requested],
                |row| row.get(0),
            )?;
            if exact_count > 1 {
                Ok((requested.to_string(), vec![requested.to_string()]))
            } else {
                Err(error)
            }
        }
    }
}

pub(crate) fn reference_epoch(query: &TruthQuery) -> i64 {
    query.as_of_epoch.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0)
    })
}

pub(super) struct MemoryRow {
    id: i64,
    topic_key: Option<String>,
    title: String,
    content: String,
    _project: String,
    branch: Option<String>,
    status: String,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    expires_at_epoch: Option<i64>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
    evidence_event_ids: Option<String>,
    source_trust_class: Option<String>,
    memory_type: String,
    owner_scope: String,
    owner_key: String,
    state_key: Option<String>,
    owner_explicit: bool,
}

fn canonical_owner_keys(conn: &Connection, rows: &[MemoryRow]) -> Result<HashMap<String, String>> {
    let mut canonical = HashMap::new();
    for row in rows {
        if row.owner_explicit
            && row.owner_scope == "repo"
            && !canonical.contains_key(&row.owner_key)
        {
            let owner = crate::project_alias::resolve_project_identity(conn, &row.owner_key)?
                .canonical_path;
            canonical.insert(row.owner_key.clone(), owner);
        }
    }
    Ok(canonical)
}

fn memory_subject_key(row: &MemoryRow, canonical_owners: &HashMap<String, String>) -> String {
    if !row.owner_explicit {
        return row.topic_key.clone().unwrap_or_else(|| memory_ref(row.id));
    }
    let logical_key = row
        .state_key
        .as_deref()
        .or(row.topic_key.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| memory_ref(row.id));
    let logical_owner = canonical_owners
        .get(&row.owner_key)
        .map(String::as_str)
        .unwrap_or(&row.owner_key);
    format!(
        "state:{}:{logical_owner}:{}:{logical_key}",
        row.owner_scope, row.memory_type
    )
}

fn memory_claim_view(
    conn: &Connection,
    row: MemoryRow,
    canonical_scope: &str,
    canonical_owners: &HashMap<String, String>,
) -> Result<ClaimView> {
    let subject_key = memory_subject_key(&row, canonical_owners);
    let mut evidence = captured_event_evidence(conn, row.evidence_event_ids.as_deref())?;
    if let Some(extra) = trust_class_evidence(row.id, row.source_trust_class.as_deref()) {
        evidence.push(extra);
    }
    let effective_valid_to = match (row.valid_to_epoch, row.expires_at_epoch) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    Ok(ClaimView {
        canonical_ref: memory_ref(row.id),
        source: ClaimSource::Memory,
        subject_key,
        statement: format!("{}: {}", row.title, row.content),
        scope: canonical_scope.to_string(),
        branch: row.branch,
        lifecycle: memory_lifecycle(&row.status),
        valid_from_epoch: row.valid_from_epoch,
        valid_to_epoch: effective_valid_to,
        created_at_epoch: row.created_at_epoch,
        updated_at_epoch: row.updated_at_epoch,
        evidence,
    })
}

/// Resolve `memories.evidence_event_ids` against the immutable
/// `captured_events` ledger. Trust is derived from who authored the event:
/// user-authored and tool-produced events are verifiable; everything else is
/// model-generated.
fn captured_event_evidence(
    conn: &Connection,
    evidence_event_ids: Option<&str>,
) -> Result<Vec<EvidenceView>> {
    let Some(raw) = evidence_event_ids else {
        return Ok(Vec::new());
    };
    let ids: Vec<i64> = match serde_json::from_str(raw) {
        Ok(ids) => ids,
        // Malformed provenance is treated as no ledger evidence, not a crash;
        // the claim then competes at model-generated tier.
        Err(_) => return Ok(Vec::new()),
    };
    let mut stmt = conn
        .prepare(
            "SELECT id, event_type, role, tool_name, created_at_epoch
             FROM captured_events WHERE id = ?1",
        )
        .context("prepare captured_events evidence lookup")?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row = stmt
            .query_row([id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .map(Some)
            .or_else(|err| match err {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .context("read captured_events evidence row")?;
        let Some((event_id, event_type, role, tool_name, created_at)) = row else {
            continue;
        };
        let trust = match (role.as_deref(), tool_name.as_deref()) {
            (Some("user"), _) => EvidenceTrust::Verified,
            (Some("tool"), _) | (_, Some(_)) => EvidenceTrust::Verified,
            _ => EvidenceTrust::ModelGenerated,
        };
        out.push(EvidenceView {
            evidence_ref: format!("captured_event:{event_id}"),
            kind: EvidenceKind::CapturedEvent,
            source_ref: match role {
                Some(role) => format!("{event_type}/{role}"),
                None => event_type,
            },
            observed_at_epoch: Some(created_at),
            trust,
        });
    }
    Ok(out)
}

/// Claim-level trust signal from `memories.source_trust_class`.
///
/// Only the extremes carry information: `user_prompt` is user-authored
/// (verified) and `external_content` is untrusted. The default
/// `local_tool_output` and `repo_file` classes add no evidence entry, so a
/// memory without ledger evidence competes at model-generated tier (its
/// content is LLM-extracted).
fn trust_class_evidence(memory_id: i64, class: Option<&str>) -> Option<EvidenceView> {
    let class = class?;
    let trust = match class {
        "user_prompt" => EvidenceTrust::Verified,
        "external_content" => EvidenceTrust::Untrusted,
        _ => return None,
    };
    Some(EvidenceView {
        evidence_ref: format!("memory_trust_class:{memory_id}"),
        kind: EvidenceKind::SourceTrustClass,
        source_ref: class.to_string(),
        observed_at_epoch: None,
        trust,
    })
}

/// Relations from `memory_edges` and trusted memory-to-memory `graph_edges`.
///
/// `memory_edges` stores replacements as `(from=old, to=new)`, while the graph
/// contract stores `supersedes` as `(from=current, to=old)`. Both are
/// normalized to "`from_ref` supersedes `to_ref`". Diagnostic-hint graph edges
/// never enter the truth projection.
fn load_memory_relations(
    conn: &Connection,
    memory_ids: &[i64],
    live_context: bool,
) -> Result<Vec<RelationView>> {
    if memory_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids_json = serde_json::to_string(memory_ids)?;
    let mut relations = Vec::new();
    let endpoint_filter = if live_context {
        "from_memory_id IN (SELECT value FROM json_each(?1))
         AND to_memory_id IN (SELECT value FROM json_each(?1))"
    } else {
        "(from_memory_id IN (SELECT value FROM json_each(?1))
          OR to_memory_id IN (SELECT value FROM json_each(?1)))"
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, edge_type, from_memory_id, to_memory_id, created_at_epoch
             FROM memory_edges
             WHERE from_memory_id IS NOT NULL AND to_memory_id IS NOT NULL
               AND ({endpoint_filter})
             ORDER BY created_at_epoch ASC, id ASC"
        ))
        .context("prepare memory_edges truth query")?;
    let edge_rows = stmt
        .query_map([&ids_json], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .context("query memory_edges for truth projection")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read memory_edges rows")?;
    for (id, edge_type, from_id, to_id, created_at) in edge_rows {
        let Some(view) = replacement_relation(
            format!("memory_edge:{id}"),
            &edge_type,
            from_id,
            to_id,
            created_at,
            None,
            None,
        ) else {
            continue;
        };
        relations.push(view);
    }

    let graph_endpoint_filter = if live_context {
        "from_node_id IN (SELECT value FROM json_each(?1))
         AND to_node_id IN (SELECT value FROM json_each(?1))"
    } else {
        "(from_node_id IN (SELECT value FROM json_each(?1))
          OR to_node_id IN (SELECT value FROM json_each(?1)))"
    };
    let mut stmt = conn
        .prepare(&format!(
            "SELECT id, edge_type, from_node_id, to_node_id, created_at_epoch,
                    valid_from_epoch, valid_to_epoch
             FROM graph_edges
             WHERE edge_trust = 'trusted'
               AND from_node_kind = 'memory' AND to_node_kind = 'memory'
               AND {graph_endpoint_filter}
             ORDER BY created_at_epoch ASC, id ASC"
        ))
        .context("prepare graph_edges truth query")?;
    let graph_rows = stmt
        .query_map([&ids_json], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })
        .context("query graph_edges for truth projection")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read graph_edges rows")?;
    for (id, edge_type, from_id, to_id, created_at, valid_from, valid_to) in graph_rows {
        let Some(view) = graph_relation(
            format!("graph_edge:{id}"),
            &edge_type,
            from_id,
            to_id,
            created_at,
            valid_from,
            valid_to,
        ) else {
            continue;
        };
        relations.push(view);
    }
    Ok(relations)
}

fn graph_relation(
    relation_ref: String,
    edge_type: &str,
    from_id: i64,
    to_id: i64,
    created_at_epoch: i64,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Option<RelationView> {
    if edge_type == "supersedes" {
        return Some(RelationView {
            relation_ref,
            kind: ClaimRelationKind::Supersedes,
            from_ref: memory_ref(from_id),
            to_ref: memory_ref(to_id),
            created_at_epoch,
            valid_from_epoch,
            valid_to_epoch,
        });
    }
    replacement_relation(
        relation_ref,
        edge_type,
        from_id,
        to_id,
        created_at_epoch,
        valid_from_epoch,
        valid_to_epoch,
    )
}

fn replacement_relation(
    relation_ref: String,
    edge_type: &str,
    from_id: i64,
    to_id: i64,
    created_at_epoch: i64,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Option<RelationView> {
    // Stored direction is (from=old, to=new) for replacement edges; the DTO
    // direction is "from_ref supersedes/derives-from to_ref", so replacement
    // kinds flip endpoints.
    let (kind, from_ref, to_ref) = match edge_type {
        "supersedes" => (
            ClaimRelationKind::Supersedes,
            memory_ref(to_id),
            memory_ref(from_id),
        ),
        "merged_into" | "split_from" | "derived_from" | "extracted_from" => (
            ClaimRelationKind::DerivedFrom,
            memory_ref(to_id),
            memory_ref(from_id),
        ),
        "conflicts" => (
            ClaimRelationKind::Refutes,
            memory_ref(from_id),
            memory_ref(to_id),
        ),
        "duplicates" => (
            ClaimRelationKind::Supports,
            memory_ref(from_id),
            memory_ref(to_id),
        ),
        _ => return None,
    };
    Some(RelationView {
        relation_ref,
        kind,
        from_ref,
        to_ref,
        created_at_epoch,
        valid_from_epoch,
        valid_to_epoch,
    })
}

/// Load user-context claims for one owner as claim views plus the explicit
/// supersedes relations recorded on the rows themselves.
pub fn load_user_claim_groups(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
) -> Result<(Vec<ClaimView>, Vec<RelationView>)> {
    let mut stmt = conn
        .prepare(
            "SELECT id, claim_type, claim_key, claim_text, status, source_kind,
                    source_refs_json, valid_from_epoch, valid_to_epoch,
                    supersedes_claim_id, created_at_epoch, updated_at_epoch
             FROM user_context_claims
             WHERE owner_scope = ?1 AND owner_key = ?2
             ORDER BY updated_at_epoch ASC, id ASC",
        )
        .context("prepare user_context_claims truth query")?;
    let rows = stmt
        .query_map([owner_scope, owner_key], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<i64>>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, i64>(11)?,
            ))
        })
        .context("query user_context_claims for truth projection")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("read user_context_claims rows")?;

    let mut claims = Vec::with_capacity(rows.len());
    let mut relations = Vec::new();
    for (
        id,
        claim_type,
        claim_key,
        claim_text,
        status,
        source_kind,
        source_refs_json,
        valid_from,
        valid_to,
        supersedes_claim_id,
        created_at,
        updated_at,
    ) in rows
    {
        claims.push(ClaimView {
            canonical_ref: user_claim_ref(id),
            source: ClaimSource::UserContextClaim,
            subject_key: format!("{claim_type}:{claim_key}"),
            statement: claim_text,
            scope: owner_key.to_string(),
            branch: None,
            lifecycle: user_claim_lifecycle(&status),
            valid_from_epoch: valid_from,
            valid_to_epoch: valid_to,
            created_at_epoch: created_at,
            updated_at_epoch: updated_at,
            evidence: user_claim_evidence(id, &source_kind, &source_refs_json),
        });
        if let Some(old_id) = supersedes_claim_id {
            relations.push(RelationView {
                relation_ref: format!("user_claim_supersedes:{id}"),
                kind: ClaimRelationKind::Supersedes,
                from_ref: user_claim_ref(id),
                to_ref: user_claim_ref(old_id),
                created_at_epoch: created_at,
                valid_from_epoch: None,
                valid_to_epoch: None,
            });
        }
    }
    Ok((claims, relations))
}

/// Source refs recorded on a user-context claim. `manual` rows are
/// user-authored (verified); `speculative_inference` and
/// `third_party_statement` are not verifiable and rank below.
fn user_claim_evidence(id: i64, source_kind: &str, source_refs_json: &str) -> Vec<EvidenceView> {
    let trust = match source_kind {
        "manual" => EvidenceTrust::Verified,
        "third_party_statement" => EvidenceTrust::Untrusted,
        _ => EvidenceTrust::ModelGenerated,
    };
    let refs: Vec<serde_json::Value> = serde_json::from_str(source_refs_json).unwrap_or_default();
    if refs.is_empty() {
        return vec![EvidenceView {
            evidence_ref: format!("user_claim_source:{id}"),
            kind: EvidenceKind::SourceRef,
            source_ref: source_kind.to_string(),
            observed_at_epoch: None,
            trust,
        }];
    }
    refs.into_iter()
        .enumerate()
        .map(|(index, value)| EvidenceView {
            evidence_ref: format!("user_claim_source:{id}:{index}"),
            kind: EvidenceKind::SourceRef,
            source_ref: match value {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            },
            observed_at_epoch: None,
            trust,
        })
        .collect()
}
