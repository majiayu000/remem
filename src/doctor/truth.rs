//! Focused, read-only diagnostics for the CurrentTruth v1 projection (GH933).

use std::collections::BTreeSet;
use std::io::{self, Write};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, Params};
use serde::Serialize;

use crate::truth::{
    project_current_truth, project_user_claim_truth, CurrentTruthProjection, Lifecycle,
    PublicationState, RetentionState, TruthQuery, TruthSelectionReason, ValidityState, Visibility,
    TRUTH_PROJECTION_VERSION,
};

const TRUTH_DOCTOR_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub(crate) struct TruthDoctorOptions {
    pub project: String,
    pub branch: Option<String>,
    pub as_of_epoch: Option<i64>,
    pub subject: Option<String>,
    pub json: bool,
    pub quiet: bool,
}

#[derive(Debug, Default, Serialize)]
struct TruthCounts {
    truth_items: usize,
    current: usize,
    contradicted: usize,
    abstentions: usize,
    rejected_claims: usize,
    evidence_refs: usize,
    supersedes_relations: usize,
    reference_issues: usize,
}

#[derive(Debug, Serialize)]
struct LifecycleMappingCount {
    object_kind: &'static str,
    stored_status: String,
    count: i64,
    publication: PublicationState,
    validity: ValidityState,
    retention: RetentionState,
    visibility: Visibility,
}

#[derive(Debug, Serialize)]
struct ConflictSummary {
    scope_kind: &'static str,
    subject_key: String,
    claim_refs: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AbstentionSummary {
    scope_kind: &'static str,
    subject_key: String,
    rejected_refs: Vec<String>,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct SupersedesLink {
    relation_ref: String,
    newer_claim_ref: String,
    older_claim_ref: String,
}

#[derive(Debug, Clone, Eq, Ord, PartialEq, PartialOrd, Serialize)]
struct ReferenceIssue {
    relation_ref: String,
    claim_ref: String,
    problem: &'static str,
    stored_status: Option<String>,
}

#[derive(Debug, Serialize)]
struct TruthDoctorReport {
    schema_version: u32,
    projection_version: u32,
    status: &'static str,
    project: String,
    branch: Option<String>,
    as_of_epoch: Option<i64>,
    subject: Option<String>,
    counts: TruthCounts,
    lifecycle_mappings: Vec<LifecycleMappingCount>,
    conflicts: Vec<ConflictSummary>,
    abstentions: Vec<AbstentionSummary>,
    supersedes: Vec<SupersedesLink>,
    reference_issues: Vec<ReferenceIssue>,
}

pub(crate) fn run_truth_doctor(opts: TruthDoctorOptions) -> Result<()> {
    let stdout = io::stdout();
    let mut sink = stdout.lock();
    run_truth_doctor_with_writer(opts, &mut sink)
}

fn run_truth_doctor_with_writer<W: Write>(opts: TruthDoctorOptions, out: &mut W) -> Result<()> {
    if opts.project.trim().is_empty() {
        bail!("doctor truth requires a non-blank project selector");
    }
    if opts
        .branch
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("doctor truth --branch must be non-blank when provided");
    }
    if opts
        .subject
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("doctor truth --subject must be non-blank when provided");
    }

    let conn = crate::db::open_db_read_only_current()
        .context("open the current remem database read-only for doctor truth")?;
    let report = build_truth_report(&conn, &opts)?;
    if opts.quiet && !opts.json {
        return Ok(());
    }
    if opts.json {
        serde_json::to_writer_pretty(&mut *out, &report)?;
        writeln!(out)?;
    } else {
        write_human(out, &report)?;
    }
    Ok(())
}

fn build_truth_report(conn: &Connection, opts: &TruthDoctorOptions) -> Result<TruthDoctorReport> {
    let query = TruthQuery {
        project: opts.project.clone(),
        branch: opts.branch.clone(),
        as_of_epoch: opts.as_of_epoch,
        subject_key: opts.subject.clone(),
    };
    let project_projection = project_current_truth(conn, &query)
        .context("project memory-backed CurrentTruth for doctor")?;
    let mut owner_projection =
        project_user_claim_truth(conn, "repo", &opts.project, opts.as_of_epoch)
            .context("project repo-owned user claims for doctor")?;
    if let Some(subject) = opts.subject.as_deref() {
        owner_projection
            .truths
            .retain(|truth| truth.subject_key == subject);
    }

    let mut conflicts = Vec::new();
    let mut abstentions = Vec::new();
    let mut counts = TruthCounts::default();
    accumulate_projection(
        "project",
        &project_projection,
        &mut counts,
        &mut conflicts,
        &mut abstentions,
    );
    accumulate_projection(
        "repo_owner",
        &owner_projection,
        &mut counts,
        &mut conflicts,
        &mut abstentions,
    );

    let lifecycle_mappings = load_lifecycle_mappings(conn, opts)?;
    let supersedes = load_supersedes_links(conn, opts)?;
    let reference_issues = load_reference_issues(conn, opts)?;
    counts.supersedes_relations = supersedes.len();
    counts.reference_issues = reference_issues.len();
    let status = if counts.contradicted > 0 || counts.reference_issues > 0 {
        "warn"
    } else {
        "ok"
    };

    Ok(TruthDoctorReport {
        schema_version: TRUTH_DOCTOR_SCHEMA_VERSION,
        projection_version: TRUTH_PROJECTION_VERSION,
        status,
        project: opts.project.clone(),
        branch: opts.branch.clone(),
        as_of_epoch: opts.as_of_epoch,
        subject: opts.subject.clone(),
        counts,
        lifecycle_mappings,
        conflicts,
        abstentions,
        supersedes,
        reference_issues,
    })
}

fn accumulate_projection(
    scope_kind: &'static str,
    projection: &CurrentTruthProjection,
    counts: &mut TruthCounts,
    conflicts: &mut Vec<ConflictSummary>,
    abstentions: &mut Vec<AbstentionSummary>,
) {
    for truth in &projection.truths {
        counts.truth_items += 1;
        counts.rejected_claims += truth.rejected.len();
        counts.evidence_refs += truth.evidence.len();
        match truth.validity {
            ValidityState::Current => counts.current += 1,
            ValidityState::Contradicted => {
                counts.contradicted += 1;
                conflicts.push(ConflictSummary {
                    scope_kind,
                    subject_key: truth.subject_key.clone(),
                    claim_refs: truth
                        .conflicting_claims
                        .iter()
                        .map(|claim| claim.canonical_ref.clone())
                        .collect(),
                });
            }
            ValidityState::Unknown
                if truth.selected_reason == TruthSelectionReason::InsufficientEvidence =>
            {
                counts.abstentions += 1;
                abstentions.push(AbstentionSummary {
                    scope_kind,
                    subject_key: truth.subject_key.clone(),
                    rejected_refs: truth.rejected.clone(),
                });
            }
            _ => {}
        }
    }
}

fn load_lifecycle_mappings(
    conn: &Connection,
    opts: &TruthDoctorOptions,
) -> Result<Vec<LifecycleMappingCount>> {
    let mut out = Vec::new();
    append_status_counts(
        conn,
        &mut out,
        "memory",
        "SELECT status, COUNT(*) FROM memories
         WHERE project = ?1 AND (?2 IS NULL OR branch IS NULL OR branch = ?2)
         GROUP BY status ORDER BY status",
        params![opts.project, opts.branch],
        crate::truth::memory_lifecycle,
    )?;
    append_status_counts(
        conn,
        &mut out,
        "observation",
        "SELECT status, COUNT(*) FROM observations
         WHERE project = ?1 AND (?2 IS NULL OR branch IS NULL OR branch = ?2)
         GROUP BY status ORDER BY status",
        params![opts.project, opts.branch],
        crate::truth::observation_lifecycle,
    )?;
    append_status_counts(
        conn,
        &mut out,
        "memory_candidate",
        "SELECT mc.review_status, COUNT(*)
         FROM memory_candidates mc
         LEFT JOIN projects p ON p.id = mc.project_id
         WHERE COALESCE(mc.target_project, mc.source_project, p.project_path) = ?1
         GROUP BY mc.review_status ORDER BY mc.review_status",
        params![opts.project],
        crate::truth::candidate_lifecycle,
    )?;
    append_status_counts(
        conn,
        &mut out,
        "user_context_claim",
        "SELECT status, COUNT(*) FROM user_context_claims
         WHERE owner_scope = 'repo' AND owner_key = ?1
         GROUP BY status ORDER BY status",
        params![opts.project],
        crate::truth::user_claim_lifecycle,
    )?;

    let reference_epoch = opts
        .as_of_epoch
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    append_status_counts(
        conn,
        &mut out,
        "trusted_graph_relation",
        "SELECT CASE
             WHEN ge.valid_to_epoch IS NOT NULL AND ge.valid_to_epoch <= ?3
             THEN 'expired' ELSE 'current' END AS relation_status,
                COUNT(*)
         FROM graph_edges ge
         JOIN memories fm ON ge.from_node_kind = 'memory' AND fm.id = ge.from_node_id
         JOIN memories tm ON ge.to_node_kind = 'memory' AND tm.id = ge.to_node_id
         WHERE ge.edge_trust = 'trusted'
           AND fm.project = ?1 AND tm.project = ?1
           AND (?2 IS NULL OR (fm.branch IS NULL OR fm.branch = ?2)
                            AND (tm.branch IS NULL OR tm.branch = ?2))
         GROUP BY relation_status ORDER BY relation_status",
        params![opts.project, opts.branch, reference_epoch],
        relation_lifecycle,
    )?;
    Ok(out)
}

fn append_status_counts<P, F>(
    conn: &Connection,
    out: &mut Vec<LifecycleMappingCount>,
    object_kind: &'static str,
    sql: &str,
    params: P,
    map: F,
) -> Result<()>
where
    P: Params,
    F: Fn(&str) -> Lifecycle,
{
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (stored_status, count) = row?;
        let lifecycle = map(&stored_status);
        out.push(LifecycleMappingCount {
            object_kind,
            stored_status,
            count,
            publication: lifecycle.publication,
            validity: lifecycle.validity,
            retention: lifecycle.retention,
            visibility: lifecycle.visibility,
        });
    }
    Ok(())
}

fn relation_lifecycle(status: &str) -> Lifecycle {
    Lifecycle {
        publication: PublicationState::Active,
        validity: if status == "expired" {
            ValidityState::Expired
        } else {
            ValidityState::Current
        },
        retention: RetentionState::Live,
        visibility: Visibility::Visible,
    }
}

fn load_supersedes_links(
    conn: &Connection,
    opts: &TruthDoctorOptions,
) -> Result<Vec<SupersedesLink>> {
    let mut links = BTreeSet::new();
    let mut stmt = conn.prepare(
        "SELECT 'memory_edge:' || me.id, 'memory:' || me.to_memory_id,
                'memory:' || me.from_memory_id
         FROM memory_edges me
         JOIN memories old ON old.id = me.from_memory_id
         JOIN memories new ON new.id = me.to_memory_id
         WHERE me.edge_type = 'supersedes'
           AND old.project = ?1 AND new.project = ?1
           AND (?2 IS NULL OR (old.branch IS NULL OR old.branch = ?2)
                            AND (new.branch IS NULL OR new.branch = ?2))",
    )?;
    let rows = stmt.query_map(params![opts.project, opts.branch], |row| {
        Ok(SupersedesLink {
            relation_ref: row.get(0)?,
            newer_claim_ref: row.get(1)?,
            older_claim_ref: row.get(2)?,
        })
    })?;
    for row in rows {
        links.insert(row?);
    }

    let mut stmt = conn.prepare(
        "SELECT 'graph_edge:' || ge.id, 'memory:' || ge.to_node_id,
                'memory:' || ge.from_node_id
         FROM graph_edges ge
         JOIN memories old ON ge.from_node_kind = 'memory' AND old.id = ge.from_node_id
         JOIN memories new ON ge.to_node_kind = 'memory' AND new.id = ge.to_node_id
         WHERE ge.edge_type = 'supersedes' AND ge.edge_trust = 'trusted'
           AND old.project = ?1 AND new.project = ?1
           AND (?2 IS NULL OR (old.branch IS NULL OR old.branch = ?2)
                            AND (new.branch IS NULL OR new.branch = ?2))",
    )?;
    let rows = stmt.query_map(params![opts.project, opts.branch], |row| {
        Ok(SupersedesLink {
            relation_ref: row.get(0)?,
            newer_claim_ref: row.get(1)?,
            older_claim_ref: row.get(2)?,
        })
    })?;
    for row in rows {
        links.insert(row?);
    }

    let mut stmt = conn.prepare(
        "SELECT 'user_claim_supersedes:' || current.id,
                'user_claim:' || current.id,
                'user_claim:' || current.supersedes_claim_id
         FROM user_context_claims current
         JOIN user_context_claims old ON old.id = current.supersedes_claim_id
         WHERE current.owner_scope = 'repo' AND current.owner_key = ?1
           AND old.owner_scope = current.owner_scope AND old.owner_key = current.owner_key",
    )?;
    let rows = stmt.query_map(params![opts.project], |row| {
        Ok(SupersedesLink {
            relation_ref: row.get(0)?,
            newer_claim_ref: row.get(1)?,
            older_claim_ref: row.get(2)?,
        })
    })?;
    for row in rows {
        links.insert(row?);
    }
    Ok(links.into_iter().collect())
}

fn load_reference_issues(
    conn: &Connection,
    opts: &TruthDoctorOptions,
) -> Result<Vec<ReferenceIssue>> {
    let mut issues = BTreeSet::new();
    collect_noncurrent_memory_edge_refs(conn, opts, &mut issues)?;
    collect_noncurrent_graph_edge_refs(conn, opts, &mut issues)?;
    collect_dangling_graph_edge_refs(conn, opts, &mut issues)?;
    collect_noncurrent_user_claim_refs(conn, opts, &mut issues)?;
    Ok(issues.into_iter().collect())
}

fn collect_noncurrent_memory_edge_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
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
         WHERE old.project = ?1 AND new.project = ?1 AND endpoint.status != 'active'
           AND NOT (me.edge_type = 'supersedes'
                    AND endpoint.id = me.from_memory_id
                    AND endpoint.status = 'superseded')
           AND (?2 IS NULL OR (old.branch IS NULL OR old.branch = ?2)
                            AND (new.branch IS NULL OR new.branch = ?2))",
        params![opts.project, opts.branch],
        "references_noncurrent_claim",
    )
}

fn collect_noncurrent_graph_edge_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    collect_reference_rows(
        conn,
        out,
        "SELECT 'graph_edge:' || ge.id, 'memory:' || endpoint.id, endpoint.status
         FROM graph_edges ge
         JOIN memories old ON ge.from_node_kind = 'memory' AND old.id = ge.from_node_id
         JOIN memories new ON ge.to_node_kind = 'memory' AND new.id = ge.to_node_id
         JOIN memories endpoint ON endpoint.id IN (ge.from_node_id, ge.to_node_id)
         WHERE ge.edge_trust = 'trusted'
           AND old.project = ?1 AND new.project = ?1 AND endpoint.status != 'active'
           AND NOT (ge.edge_type = 'supersedes'
                    AND endpoint.id = ge.from_node_id
                    AND endpoint.status = 'superseded')
           AND (?2 IS NULL OR (old.branch IS NULL OR old.branch = ?2)
                            AND (new.branch IS NULL OR new.branch = ?2))",
        params![opts.project, opts.branch],
        "references_noncurrent_claim",
    )
}

fn collect_dangling_graph_edge_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
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
           AND ((fm.id IS NULL AND tm.project = ?1)
             OR (tm.id IS NULL AND fm.project = ?1))
           AND (?2 IS NULL OR COALESCE(fm.branch, tm.branch) IS NULL
                            OR COALESCE(fm.branch, tm.branch) = ?2)",
    )?;
    let rows = stmt.query_map(params![opts.project, opts.branch], |row| {
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

fn collect_noncurrent_user_claim_refs(
    conn: &Connection,
    opts: &TruthDoctorOptions,
    out: &mut BTreeSet<ReferenceIssue>,
) -> Result<()> {
    collect_reference_rows(
        conn,
        out,
        "SELECT 'user_claim_supersedes:' || current.id,
                'user_claim:' || old.id, old.status
         FROM user_context_claims current
         JOIN user_context_claims old ON old.id = current.supersedes_claim_id
         WHERE current.owner_scope = 'repo' AND current.owner_key = ?1
           AND old.owner_scope = current.owner_scope AND old.owner_key = current.owner_key
           AND old.status NOT IN ('active', 'superseded')",
        params![opts.project],
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

fn write_human<W: Write>(out: &mut W, report: &TruthDoctorReport) -> Result<()> {
    writeln!(out, "CurrentTruth diagnostic ({})", report.status)?;
    writeln!(out, "  project: {}", report.project)?;
    writeln!(
        out,
        "  branch: {}  as_of_epoch: {}  subject: {}",
        report.branch.as_deref().unwrap_or("all"),
        report
            .as_of_epoch
            .map(|value| value.to_string())
            .as_deref()
            .unwrap_or("now"),
        report.subject.as_deref().unwrap_or("all")
    )?;
    writeln!(
        out,
        "  truths: {} current={} contradicted={} abstentions={} rejected={}",
        report.counts.truth_items,
        report.counts.current,
        report.counts.contradicted,
        report.counts.abstentions,
        report.counts.rejected_claims
    )?;
    writeln!(
        out,
        "  evidence_refs: {} supersedes={} reference_issues={}",
        report.counts.evidence_refs,
        report.counts.supersedes_relations,
        report.counts.reference_issues
    )?;
    writeln!(out, "  lifecycle mappings:")?;
    for mapping in &report.lifecycle_mappings {
        writeln!(
            out,
            "    {}.{}={} -> {:?}/{:?}/{:?}/{:?}",
            mapping.object_kind,
            mapping.stored_status,
            mapping.count,
            mapping.publication,
            mapping.validity,
            mapping.retention,
            mapping.visibility
        )?;
    }
    for conflict in &report.conflicts {
        writeln!(
            out,
            "  conflict [{}] {}: {}",
            conflict.scope_kind,
            conflict.subject_key,
            conflict.claim_refs.join(", ")
        )?;
    }
    for abstention in &report.abstentions {
        writeln!(
            out,
            "  abstention [{}] {}: {}",
            abstention.scope_kind,
            abstention.subject_key,
            abstention.rejected_refs.join(", ")
        )?;
    }
    for link in &report.supersedes {
        writeln!(
            out,
            "  supersedes {}: {} -> {}",
            link.relation_ref, link.newer_claim_ref, link.older_claim_ref
        )?;
    }
    for issue in &report.reference_issues {
        writeln!(
            out,
            "  reference issue {}: {} {} ({})",
            issue.relation_ref,
            issue.problem,
            issue.claim_ref,
            issue.stored_status.as_deref().unwrap_or("missing")
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    fn insert_memory(
        conn: &Connection,
        id: i64,
        topic: &str,
        status: &str,
        updated_at: i64,
    ) -> Result<()> {
        conn.execute(
            "INSERT INTO memories
             (id, project, topic_key, title, content, memory_type,
              created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', ?2, 'Decision', ?2, 'decision', 1, ?3, ?4)",
            params![id, topic, updated_at, status],
        )?;
        Ok(())
    }

    fn options() -> TruthDoctorOptions {
        TruthDoctorOptions {
            project: "/repo".to_string(),
            branch: None,
            as_of_epoch: Some(100),
            subject: None,
            json: true,
            quiet: false,
        }
    }

    #[test]
    fn report_surfaces_conflicts_supersedes_and_noncurrent_references() -> Result<()> {
        let conn = test_conn()?;
        insert_memory(&conn, 1, "deploy", "superseded", 10)?;
        insert_memory(&conn, 2, "deploy", "active", 20)?;
        conn.execute(
            "INSERT INTO memory_edges
             (edge_type, from_memory_id, to_memory_id, created_at_epoch)
             VALUES ('supersedes', 1, 2, 20)",
            [],
        )?;
        conn.execute(
            "INSERT INTO memory_edges
             (edge_type, from_memory_id, to_memory_id, created_at_epoch)
             VALUES ('duplicates', 1, 2, 21)",
            [],
        )?;
        let before = conn.total_changes();

        let report = build_truth_report(&conn, &options())?;

        assert_eq!(
            conn.total_changes(),
            before,
            "diagnostic must be SELECT-only"
        );
        assert_eq!(report.counts.truth_items, 1);
        assert_eq!(report.counts.current, 1);
        assert_eq!(report.counts.supersedes_relations, 1);
        assert_eq!(report.counts.reference_issues, 1);
        assert!(report.reference_issues.iter().any(|issue| {
            issue.claim_ref == "memory:1" && issue.stored_status.as_deref() == Some("superseded")
        }));
        assert!(report.lifecycle_mappings.iter().any(|mapping| {
            mapping.object_kind == "memory"
                && mapping.stored_status == "superseded"
                && mapping.validity == ValidityState::Superseded
        }));
        Ok(())
    }

    #[test]
    fn unresolved_equal_claims_are_reported_without_claim_text() -> Result<()> {
        let conn = test_conn()?;
        insert_memory(&conn, 1, "runtime", "active", 20)?;
        insert_memory(&conn, 2, "runtime", "active", 20)?;

        let report = build_truth_report(&conn, &options())?;
        let encoded = serde_json::to_string(&report)?;

        assert_eq!(report.status, "warn");
        assert_eq!(report.counts.contradicted, 1);
        assert_eq!(report.conflicts[0].claim_refs, ["memory:1", "memory:2"]);
        assert!(!encoded.contains("Decision: runtime"));
        Ok(())
    }
}
