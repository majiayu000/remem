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

use super::types::DoctorOutcome;

#[path = "truth_reference.rs"]
mod reference_diagnostics;
use reference_diagnostics::load_reference_issues;
#[path = "truth_scope.rs"]
mod scope;
use scope::DoctorScope;

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

pub(crate) fn run_truth_doctor(opts: TruthDoctorOptions) -> Result<DoctorOutcome> {
    let stdout = io::stdout();
    let mut sink = stdout.lock();
    run_truth_doctor_with_writer(opts, &mut sink)
}

fn run_truth_doctor_with_writer<W: Write>(
    opts: TruthDoctorOptions,
    out: &mut W,
) -> Result<DoctorOutcome> {
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
    let outcome = truth_outcome(&report);
    if opts.quiet && !opts.json {
        return Ok(outcome);
    }
    if opts.json {
        serde_json::to_writer_pretty(&mut *out, &report)?;
        writeln!(out)?;
    } else {
        write_human(out, &report)?;
    }
    Ok(outcome)
}

fn truth_outcome(report: &TruthDoctorReport) -> DoctorOutcome {
    DoctorOutcome {
        fails: 0,
        warns: usize::from(report.status == "warn"),
    }
}

fn build_truth_report(conn: &Connection, opts: &TruthDoctorOptions) -> Result<TruthDoctorReport> {
    let snapshot = conn
        .unchecked_transaction()
        .context("begin read snapshot for doctor truth")?;
    let report = build_truth_report_from_snapshot(&snapshot, opts)?;
    snapshot
        .commit()
        .context("finish read snapshot for doctor truth")?;
    Ok(report)
}

fn build_truth_report_from_snapshot(
    conn: &Connection,
    opts: &TruthDoctorOptions,
) -> Result<TruthDoctorReport> {
    let reference_epoch = opts
        .as_of_epoch
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let query = TruthQuery {
        project: opts.project.clone(),
        branch: opts.branch.clone(),
        as_of_epoch: Some(reference_epoch),
        subject_key: None,
    };
    let scope = DoctorScope::load(conn, &query, opts.subject.as_deref())?;
    let mut project_projection = project_current_truth(conn, &query)
        .context("project memory-backed CurrentTruth for doctor")?;
    project_projection
        .truths
        .retain(|truth| scope.keeps_subject(&truth.subject_key));
    let mut owner_projection =
        project_user_claim_truth(conn, "repo", &opts.project, Some(reference_epoch))
            .context("project repo-owned user claims for doctor")?;
    if let Some(subject) = opts.subject.as_deref() {
        owner_projection
            .truths
            .retain(|truth| user_claim_subject_matches(&truth.subject_key, subject));
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

    let lifecycle_mappings = load_lifecycle_mappings(conn, opts, &scope, reference_epoch)?;
    let supersedes = load_supersedes_links(conn, opts, &scope, reference_epoch)?;
    let reference_issues = load_reference_issues(conn, opts, &scope, reference_epoch)?;
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
        as_of_epoch: Some(reference_epoch),
        subject: opts.subject.clone(),
        counts,
        lifecycle_mappings,
        conflicts,
        abstentions,
        supersedes,
        reference_issues,
    })
}

fn user_claim_subject_matches(composite_key: &str, selector: &str) -> bool {
    composite_key == selector
        || composite_key
            .split_once(':')
            .is_some_and(|(_, claim_key)| claim_key == selector)
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
    scope: &DoctorScope,
    reference_epoch: i64,
) -> Result<Vec<LifecycleMappingCount>> {
    let mut out = Vec::new();
    append_status_counts(
        conn,
        &mut out,
        "memory",
        "SELECT status, COUNT(*) FROM memories
         WHERE id IN (SELECT value FROM json_each(?1))
           AND created_at_epoch <= ?2
         GROUP BY status ORDER BY status",
        params![scope.memory_ids_json, reference_epoch],
        crate::truth::memory_lifecycle,
    )?;
    append_status_counts(
        conn,
        &mut out,
        "observation",
        "SELECT status, COUNT(*) FROM observations
         WHERE project IN (SELECT value FROM json_each(?1))
           AND (?2 IS NULL OR branch IS NULL OR branch = ?2)
           AND created_at_epoch <= ?3
           AND ?4 IS NULL
         GROUP BY status ORDER BY status",
        params![
            scope.projects_json,
            opts.branch,
            reference_epoch,
            opts.subject
        ],
        crate::truth::observation_lifecycle,
    )?;
    append_status_counts(
        conn,
        &mut out,
        "memory_candidate",
        "SELECT mc.review_status, COUNT(*)
         FROM memory_candidates mc
         LEFT JOIN projects p ON p.id = mc.project_id
         WHERE COALESCE(mc.target_project, mc.source_project, p.project_path)
               IN (SELECT value FROM json_each(?1))
           AND mc.created_at_epoch <= ?2
           AND (?3 IS NULL OR mc.topic_key IN (SELECT value FROM json_each(?3)))
         GROUP BY mc.review_status ORDER BY mc.review_status",
        params![
            scope.projects_json,
            reference_epoch,
            scope.candidate_topic_keys_json
        ],
        crate::truth::candidate_lifecycle,
    )?;
    append_status_counts(
        conn,
        &mut out,
        "user_context_claim",
        "SELECT status, COUNT(*) FROM user_context_claims
         WHERE owner_scope = 'repo' AND owner_key = ?1
           AND created_at_epoch <= ?2
           AND (?3 IS NULL
                OR claim_key = ?3
                OR claim_type || ':' || claim_key = ?3)
         GROUP BY status ORDER BY status",
        params![opts.project, reference_epoch, opts.subject],
        crate::truth::user_claim_lifecycle,
    )?;

    append_status_counts(
        conn,
        &mut out,
        "trusted_graph_relation",
        "SELECT CASE
             WHEN ge.valid_to_epoch IS NOT NULL AND ge.valid_to_epoch <= ?2
             THEN 'expired' ELSE 'current' END AS relation_status,
                COUNT(*)
         FROM graph_edges ge
         JOIN memories fm ON ge.from_node_kind = 'memory' AND fm.id = ge.from_node_id
         JOIN memories tm ON ge.to_node_kind = 'memory' AND tm.id = ge.to_node_id
         WHERE ge.edge_trust = 'trusted'
           AND (fm.id IN (SELECT value FROM json_each(?1))
             OR tm.id IN (SELECT value FROM json_each(?1)))
           AND ge.created_at_epoch <= ?2
           AND (ge.valid_from_epoch IS NULL OR ge.valid_from_epoch <= ?2)
         GROUP BY relation_status ORDER BY relation_status",
        params![scope.memory_ids_json, reference_epoch],
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
    scope: &DoctorScope,
    reference_epoch: i64,
) -> Result<Vec<SupersedesLink>> {
    let mut links = BTreeSet::new();
    let mut stmt = conn.prepare(
        "SELECT 'memory_edge:' || me.id, 'memory:' || me.to_memory_id,
                'memory:' || me.from_memory_id
         FROM memory_edges me
         JOIN memories old ON old.id = me.from_memory_id
         JOIN memories new ON new.id = me.to_memory_id
         WHERE me.edge_type = 'supersedes'
           AND (old.id IN (SELECT value FROM json_each(?1))
             OR new.id IN (SELECT value FROM json_each(?1)))
           AND me.created_at_epoch <= ?2",
    )?;
    let rows = stmt.query_map(params![scope.memory_ids_json, reference_epoch], |row| {
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
        "SELECT 'graph_edge:' || ge.id, 'memory:' || ge.from_node_id,
                'memory:' || ge.to_node_id
         FROM graph_edges ge
         JOIN memories new ON ge.from_node_kind = 'memory' AND new.id = ge.from_node_id
         JOIN memories old ON ge.to_node_kind = 'memory' AND old.id = ge.to_node_id
         WHERE ge.edge_type = 'supersedes' AND ge.edge_trust = 'trusted'
           AND (old.id IN (SELECT value FROM json_each(?1))
             OR new.id IN (SELECT value FROM json_each(?1)))
           AND ge.created_at_epoch <= ?2
           AND (ge.valid_from_epoch IS NULL OR ge.valid_from_epoch <= ?2)
           AND (ge.valid_to_epoch IS NULL OR ge.valid_to_epoch > ?2)",
    )?;
    let rows = stmt.query_map(params![scope.memory_ids_json, reference_epoch], |row| {
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
           AND old.owner_scope = current.owner_scope AND old.owner_key = current.owner_key
           AND current.created_at_epoch <= ?2
           AND (?3 IS NULL
                OR current.claim_key = ?3
                OR current.claim_type || ':' || current.claim_key = ?3
                OR old.claim_key = ?3
                OR old.claim_type || ':' || old.claim_key = ?3)",
    )?;
    let rows = stmt.query_map(
        params![opts.project, reference_epoch, opts.subject],
        |row| {
            Ok(SupersedesLink {
                relation_ref: row.get(0)?,
                newer_claim_ref: row.get(1)?,
                older_claim_ref: row.get(2)?,
            })
        },
    )?;
    for row in rows {
        links.insert(row?);
    }
    Ok(links.into_iter().collect())
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
#[path = "truth_tests.rs"]
mod tests;
