use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use super::{
    claims::DEFAULT_OWNER_KEY,
    summary::{self, SummaryRequest, UserContextSummary},
};

const MAX_SNAPSHOT_CLAIMS: usize = 500;

#[derive(Debug, Clone)]
pub struct ProfileSnapshotRequest<'a> {
    pub project: &'a str,
    pub owner_scope: &'a str,
    pub owner_key: Option<&'a str>,
    pub source_of_truth: &'a Path,
    pub include_suppressed: bool,
    pub include_sensitive: bool,
    pub include_inactive: bool,
    pub include_deleted: bool,
    pub include_manual_summaries: bool,
}

#[derive(Debug)]
struct SnapshotOwner {
    scope: String,
    key: String,
}

#[derive(Debug)]
struct SnapshotClaim {
    id: i64,
    claim_type: String,
    claim_key: String,
    claim_text: String,
    owner_scope: String,
    owner_key: String,
    sensitivity: String,
    source_kind: String,
    source_refs_json: String,
    status: String,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    updated_at_epoch: i64,
}

#[derive(Debug)]
struct SnapshotSummary {
    summary: UserContextSummary,
    reasons: Vec<String>,
}

pub fn render_markdown_profile_snapshot(
    conn: &Connection,
    req: &ProfileSnapshotRequest<'_>,
) -> Result<String> {
    let owner = normalize_snapshot_owner(req.owner_scope, req.owner_key, req.project)?;
    let mode = audit_mode_label(req);
    let generated = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut output = String::new();
    output.push_str("# remem User Profile Snapshot\n\n");
    output.push_str(&format!("Generated: {generated}\n"));
    output.push_str(&format!(
        "Effective owners: {}:{}, repo:{}\n",
        owner.scope, owner.key, req.project
    ));
    output.push_str(&format!("Project: {}\n", req.project));
    output.push_str(&format!(
        "Source of truth: SQLite database at {}\n",
        req.source_of_truth.display()
    ));
    output.push_str(&format!("Mode: {mode}\n"));
    output.push_str(
        "Derived snapshot: editing this Markdown does not mutate remem state; use remem user commands to change SQLite-backed user context.\n\n",
    );

    render_summary_section(conn, req, &owner, &mut output)?;
    render_claim_sections(conn, req, &owner, &mut output)?;
    if !output.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

fn render_summary_section(
    conn: &Connection,
    req: &ProfileSnapshotRequest<'_>,
    owner: &SnapshotOwner,
    output: &mut String,
) -> Result<()> {
    output.push_str("## Active Summary\n\n");
    match load_snapshot_summary(conn, req, owner)? {
        Some(snapshot) if snapshot.reasons.is_empty() => {
            output.push_str(&snapshot.summary.summary_text);
            output.push_str("\n\n");
            render_summary_metadata(
                &snapshot.summary,
                summary_provenance(&snapshot.summary),
                output,
            );
        }
        Some(snapshot) => {
            output.push_str("No default-eligible active summary text.\n\n");
            if should_show_excluded_summary(req, &snapshot.reasons) {
                output.push_str("## Excluded Summary\n\n");
                if summary_text_allowed(req, &snapshot.reasons) {
                    output.push_str(&snapshot.summary.summary_text);
                    output.push_str("\n\n");
                } else {
                    output.push_str("- [summary:");
                    output.push_str(&snapshot.summary.id.to_string());
                    output.push_str("] text redacted\n");
                }
                output.push_str(&format!("  - reason: {}\n", snapshot.reasons.join(", ")));
                render_summary_metadata(
                    &snapshot.summary,
                    summary_provenance(&snapshot.summary),
                    output,
                );
            }
        }
        None => output.push_str("No active summary found.\n\n"),
    }
    Ok(())
}

fn render_claim_sections(
    conn: &Connection,
    req: &ProfileSnapshotRequest<'_>,
    owner: &SnapshotOwner,
    output: &mut String,
) -> Result<()> {
    let claims = load_snapshot_claims(conn, owner, req.project)?;
    let now = Utc::now().timestamp();
    let mut active = Vec::new();
    let mut excluded = Vec::new();
    for claim in claims {
        let reasons = claim_exclusion_reasons(conn, &claim, now)?;
        if reasons.is_empty() {
            if active.len() < MAX_SNAPSHOT_CLAIMS {
                active.push(claim);
            }
        } else if should_show_excluded_claim(req, &reasons) && excluded.len() < MAX_SNAPSHOT_CLAIMS
        {
            excluded.push((claim, reasons));
        }
    }

    output.push_str("## Active Claims\n\n");
    if active.is_empty() {
        output.push_str("No active default-eligible claims found.\n\n");
    } else {
        for claim in &active {
            render_claim(claim, None, true, output);
        }
        output.push('\n');
    }

    output.push_str("## Sources\n\n");
    if active.is_empty() {
        output.push_str("- active_claim_ids=[]\n\n");
    } else {
        let ids = active
            .iter()
            .map(|claim| claim.id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        output.push_str(&format!("- active_claim_ids=[{ids}]\n\n"));
    }

    output.push_str("## Excluded From Default Use\n\n");
    if excluded.is_empty() {
        if any_audit_flag(req) {
            output.push_str("No excluded items matched the requested audit flags.\n");
        } else {
            output.push_str("No excluded items shown. Re-run with audit flags to inspect them.\n");
        }
    } else {
        for (claim, reasons) in &excluded {
            let render_text = claim_text_allowed(req, reasons);
            render_claim(claim, Some(reasons), render_text, output);
        }
    }
    Ok(())
}

fn render_summary_metadata(summary: &UserContextSummary, provenance: &str, output: &mut String) {
    output.push_str(&format!("- summary:{}\n", summary.id));
    output.push_str(&format!("  - provenance: {provenance}\n"));
    output.push_str(&format!(
        "  - owner: {}:{}\n",
        summary.owner_scope, summary.owner_key
    ));
    output.push_str(&format!(
        "  - source_claim_ids: [{}]\n",
        join_i64s(&summary.source_claim_ids)
    ));
    output.push_str(&format!(
        "  - source_memory_ids: [{}]\n",
        join_i64s(&summary.source_memory_ids)
    ));
    output.push_str(&format!(
        "  - activity_refs: [{}]\n\n",
        summary
            .source_activity_refs
            .iter()
            .map(|activity| format!("{}:{}", activity.kind, activity.id))
            .collect::<Vec<_>>()
            .join(", ")
    ));
}

fn summary_provenance(summary: &UserContextSummary) -> &'static str {
    if summary.model.as_deref() == Some("manual-edit") {
        "manual-edit"
    } else if summary.source_claim_ids.is_empty()
        && summary.source_memory_ids.is_empty()
        && summary.source_activity_refs.is_empty()
    {
        "unsourced"
    } else {
        "source-supported"
    }
}

fn render_claim(
    claim: &SnapshotClaim,
    reasons: Option<&[String]>,
    render_text: bool,
    output: &mut String,
) {
    let text = if render_text {
        compact_snapshot_line(&claim.claim_text)
    } else {
        "[redacted]".to_string()
    };
    let claim_key = if render_text {
        compact_snapshot_line(&claim.claim_key)
    } else {
        "[redacted]".to_string()
    };
    output.push_str(&format!(
        "- [claim:{}] {}:{} - {}\n",
        claim.id, claim.claim_type, claim_key, text
    ));
    output.push_str(&format!(
        "  - owner: {}:{}\n",
        claim.owner_scope, claim.owner_key
    ));
    output.push_str(&format!("  - sensitivity: {}\n", claim.sensitivity));
    output.push_str(&format!("  - status: {}\n", claim.status));
    output.push_str(&format!("  - source: {}\n", claim.source_kind));
    output.push_str(&format!("  - source_refs: {}\n", claim.source_refs_json));
    output.push_str(&format!(
        "  - updated: {}\n",
        format_snapshot_epoch(claim.updated_at_epoch)
    ));
    if let Some(reasons) = reasons {
        output.push_str(&format!("  - reason: {}\n", reasons.join(", ")));
    }
}

fn load_snapshot_summary(
    conn: &Connection,
    req: &ProfileSnapshotRequest<'_>,
    owner: &SnapshotOwner,
) -> Result<Option<SnapshotSummary>> {
    let summary_req = SummaryRequest {
        owner_scope: Some(&owner.scope),
        owner_key: Some(&owner.key),
        project: req.project,
    };
    let Some(summary) = summary::load_active_summary_unfiltered(conn, &summary_req)? else {
        return Ok(None);
    };
    let mut reasons = Vec::new();
    let suppressed = summary::summary_is_policy_suppressed(conn, &summary)?;
    if suppressed {
        push_reason(&mut reasons, "suppressed");
    }
    push_summary_source_reasons(conn, &summary, &mut reasons)?;
    if summary.model.as_deref() == Some("manual-edit") {
        push_reason(&mut reasons, "provenance:manual-edit");
    }
    if summary.source_claim_ids.is_empty()
        && summary.source_memory_ids.is_empty()
        && summary.source_activity_refs.is_empty()
    {
        push_reason(&mut reasons, "provenance:unsourced");
    }
    Ok(Some(SnapshotSummary { summary, reasons }))
}

fn push_summary_source_reasons(
    conn: &Connection,
    summary: &UserContextSummary,
    reasons: &mut Vec<String>,
) -> Result<()> {
    let now = Utc::now().timestamp();
    for claim_id in &summary.source_claim_ids {
        if let Some(claim) = load_snapshot_claim_by_id(conn, *claim_id)? {
            for reason in claim_exclusion_reasons(conn, &claim, now)? {
                push_reason(reasons, &reason);
            }
        } else {
            push_reason(reasons, "source:missing");
        }
    }
    for memory_id in &summary.source_memory_ids {
        for reason in memory_source_exclusion_reasons(conn, *memory_id)? {
            push_reason(reasons, &reason);
        }
    }
    Ok(())
}

fn load_snapshot_claim_by_id(conn: &Connection, id: i64) -> Result<Option<SnapshotClaim>> {
    conn.query_row(
        "SELECT id, claim_type, claim_key, claim_text, owner_scope, owner_key,
                sensitivity, source_kind, source_refs_json, status,
                valid_from_epoch, valid_to_epoch, updated_at_epoch
         FROM user_context_claims
         WHERE id = ?1",
        [id],
        |row| {
            Ok(SnapshotClaim {
                id: row.get(0)?,
                claim_type: row.get(1)?,
                claim_key: row.get(2)?,
                claim_text: row.get(3)?,
                owner_scope: row.get(4)?,
                owner_key: row.get(5)?,
                sensitivity: row.get(6)?,
                source_kind: row.get(7)?,
                source_refs_json: row.get(8)?,
                status: row.get(9)?,
                valid_from_epoch: row.get(10)?,
                valid_to_epoch: row.get(11)?,
                updated_at_epoch: row.get(12)?,
            })
        },
    )
    .optional()
    .context("load user-context summary source claim")
}

fn memory_source_exclusion_reasons(conn: &Connection, memory_id: i64) -> Result<Vec<String>> {
    let Some((status, expires_at_epoch)) = conn
        .query_row(
            "SELECT status, expires_at_epoch
             FROM memories
             WHERE id = ?1",
            [memory_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .optional()
        .context("load user-context summary source memory")?
    else {
        return Ok(vec!["source:missing".to_string()]);
    };
    let mut reasons = Vec::new();
    match status.as_str() {
        "active" => {}
        "deleted" => push_reason(&mut reasons, "status:deleted"),
        other => push_reason(&mut reasons, &format!("status:{other}")),
    }
    if expires_at_epoch.is_some_and(|epoch| epoch <= Utc::now().timestamp()) {
        push_reason(&mut reasons, "validity:expired");
    }
    if memory_source_is_superseded(conn, memory_id)? {
        push_reason(&mut reasons, "status:superseded");
    }
    if memory_source_is_policy_suppressed(conn, memory_id)? {
        push_reason(&mut reasons, "suppressed");
    }
    Ok(reasons)
}

fn memory_source_is_superseded(conn: &Connection, memory_id: i64) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memory_edges
         WHERE edge_type = 'supersedes'
           AND from_memory_id = ?1",
        [memory_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn memory_source_is_policy_suppressed(conn: &Connection, memory_id: i64) -> Result<bool> {
    let sql = format!(
        "SELECT COUNT(*)
         FROM memories
         WHERE id = ?1
           AND {}",
        crate::memory::suppression::memory_policy_filter_sql("memories"),
    );
    let visible_count: i64 = conn.query_row(&sql, [memory_id], |row| row.get(0))?;
    Ok(visible_count == 0)
}

fn load_snapshot_claims(
    conn: &Connection,
    owner: &SnapshotOwner,
    project: &str,
) -> Result<Vec<SnapshotClaim>> {
    let mut stmt = conn.prepare(
        "SELECT id, claim_type, claim_key, claim_text, owner_scope, owner_key,
                sensitivity, source_kind, source_refs_json, status,
                valid_from_epoch, valid_to_epoch, updated_at_epoch
         FROM user_context_claims
         WHERE ((owner_scope = ?1 AND owner_key = ?2)
            OR (owner_scope = 'user' AND owner_key = ?4)
            OR (owner_scope = 'repo' AND owner_key = ?3))
         ORDER BY owner_scope ASC, owner_key ASC, claim_type ASC, claim_key ASC, id ASC",
    )?;
    let rows = stmt.query_map(
        params![owner.scope, owner.key, project, DEFAULT_OWNER_KEY],
        |row| {
            Ok(SnapshotClaim {
                id: row.get(0)?,
                claim_type: row.get(1)?,
                claim_key: row.get(2)?,
                claim_text: row.get(3)?,
                owner_scope: row.get(4)?,
                owner_key: row.get(5)?,
                sensitivity: row.get(6)?,
                source_kind: row.get(7)?,
                source_refs_json: row.get(8)?,
                status: row.get(9)?,
                valid_from_epoch: row.get(10)?,
                valid_to_epoch: row.get(11)?,
                updated_at_epoch: row.get(12)?,
            })
        },
    )?;
    crate::db::query::collect_rows(rows)
}

fn claim_exclusion_reasons(
    conn: &Connection,
    claim: &SnapshotClaim,
    now: i64,
) -> Result<Vec<String>> {
    let mut reasons = Vec::new();
    match claim.status.as_str() {
        "active" => {}
        "suppressed" => reasons.push("status:suppressed".to_string()),
        "deleted" => reasons.push("status:deleted".to_string()),
        other => reasons.push(format!("status:{other}")),
    }
    if claim.sensitivity != "normal" {
        reasons.push(format!("sensitivity:{}", claim.sensitivity));
    }
    if claim.valid_from_epoch.is_some_and(|epoch| epoch > now) {
        reasons.push("validity:future".to_string());
    }
    if claim.valid_to_epoch.is_some_and(|epoch| epoch <= now) {
        reasons.push("validity:expired".to_string());
    }
    if crate::memory::suppression::user_claim_is_policy_suppressed(conn, claim.id)? {
        push_reason(&mut reasons, "suppressed");
    }
    Ok(reasons)
}

fn normalize_snapshot_owner(
    owner_scope: &str,
    owner_key: Option<&str>,
    project: &str,
) -> Result<SnapshotOwner> {
    let scope = owner_scope.trim();
    if scope.is_empty() {
        bail!("owner scope cannot be empty");
    }
    let key = match scope {
        "user" => owner_key.unwrap_or(DEFAULT_OWNER_KEY),
        "repo" => owner_key.unwrap_or(project),
        "workspace" | "session" => owner_key
            .filter(|value| !value.trim().is_empty())
            .context("owner key is required for workspace and session profile snapshots")?,
        other => bail!("unsupported owner scope for profile snapshot: {other}"),
    };
    Ok(SnapshotOwner {
        scope: scope.to_string(),
        key: key.to_string(),
    })
}

fn should_show_excluded_claim(req: &ProfileSnapshotRequest<'_>, reasons: &[String]) -> bool {
    any_audit_flag(req)
        && reasons
            .iter()
            .any(|reason| reason_allowed_by_some_flag(req, reason))
}

fn should_show_excluded_summary(req: &ProfileSnapshotRequest<'_>, reasons: &[String]) -> bool {
    any_audit_flag(req)
        && reasons
            .iter()
            .any(|reason| summary_reason_allowed_by_some_flag(req, reason))
}

fn claim_text_allowed(req: &ProfileSnapshotRequest<'_>, reasons: &[String]) -> bool {
    reasons.iter().all(|reason| reason_allowed(req, reason))
}

fn summary_text_allowed(req: &ProfileSnapshotRequest<'_>, reasons: &[String]) -> bool {
    reasons.iter().all(|reason| match reason.as_str() {
        reason if reason.starts_with("provenance:") => req.include_manual_summaries,
        reason => reason_allowed(req, reason),
    })
}

fn reason_allowed(req: &ProfileSnapshotRequest<'_>, reason: &str) -> bool {
    match reason {
        "suppressed" | "status:suppressed" => req.include_suppressed,
        "status:deleted" => req.include_deleted,
        reason if reason.starts_with("sensitivity:") => req.include_sensitive,
        reason if inactive_status_reason(reason) => req.include_inactive,
        reason if reason.starts_with("validity:") => req.include_inactive,
        _ => false,
    }
}

fn reason_allowed_by_some_flag(req: &ProfileSnapshotRequest<'_>, reason: &str) -> bool {
    match reason {
        "suppressed" | "status:suppressed" => req.include_suppressed,
        "status:deleted" => req.include_deleted,
        reason if reason.starts_with("sensitivity:") => req.include_sensitive,
        reason if inactive_status_reason(reason) => req.include_inactive,
        reason if reason.starts_with("validity:") => req.include_inactive,
        _ => false,
    }
}

fn summary_reason_allowed_by_some_flag(req: &ProfileSnapshotRequest<'_>, reason: &str) -> bool {
    match reason {
        reason if reason.starts_with("provenance:") => req.include_manual_summaries,
        reason => reason_allowed_by_some_flag(req, reason),
    }
}

fn inactive_status_reason(reason: &str) -> bool {
    matches!(
        reason.strip_prefix("status:"),
        Some("inactive" | "stale" | "superseded" | "archived")
    )
}

fn any_audit_flag(req: &ProfileSnapshotRequest<'_>) -> bool {
    req.include_suppressed
        || req.include_sensitive
        || req.include_inactive
        || req.include_deleted
        || req.include_manual_summaries
}

fn audit_mode_label(req: &ProfileSnapshotRequest<'_>) -> String {
    let mut flags = Vec::new();
    if req.include_suppressed {
        flags.push("include_suppressed");
    }
    if req.include_sensitive {
        flags.push("include_sensitive");
    }
    if req.include_inactive {
        flags.push("include_inactive");
    }
    if req.include_deleted {
        flags.push("include_deleted");
    }
    if req.include_manual_summaries {
        flags.push("include_manual_summaries");
    }
    if flags.is_empty() {
        "default".to_string()
    } else {
        format!("audit({})", flags.join(","))
    }
}

fn join_i64s(values: &[i64]) -> String {
    values
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn compact_snapshot_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn format_snapshot_epoch(epoch: i64) -> String {
    chrono::DateTime::from_timestamp(epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

fn push_reason(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}
