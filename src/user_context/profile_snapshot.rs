use std::path::Path;

use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection};

use super::{
    claims::DEFAULT_OWNER_KEY,
    summary::{self, SummaryRequest, UserContextSummary},
};

const MAX_SNAPSHOT_CLAIMS: i64 = 500;

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
            render_summary_metadata(&snapshot.summary, "source-supported", output);
        }
        Some(snapshot) => {
            output.push_str("No default-eligible active summary text.\n\n");
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
            render_summary_metadata(&snapshot.summary, "excluded", output);
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
            active.push(claim);
        } else if should_show_excluded_claim(req, &reasons) {
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
    output.push_str(&format!(
        "- [claim:{}] {}:{} - {}\n",
        claim.id, claim.claim_type, claim.claim_key, text
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
    let visible = summary::load_active_summary(conn, &summary_req)?;
    let mut reasons = Vec::new();
    if visible.as_ref().map(|item| item.id) != Some(summary.id) {
        reasons.push("source:not-default-visible".to_string());
    }
    if summary::summary_is_policy_suppressed(conn, &summary)? {
        push_reason(&mut reasons, "suppressed");
    }
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
            OR (owner_scope = 'repo' AND owner_key = ?3))
         ORDER BY owner_scope ASC, owner_key ASC, claim_type ASC, claim_key ASC, id ASC
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![owner.scope, owner.key, project, MAX_SNAPSHOT_CLAIMS],
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

fn claim_text_allowed(req: &ProfileSnapshotRequest<'_>, reasons: &[String]) -> bool {
    reasons.iter().all(|reason| reason_allowed(req, reason))
}

fn summary_text_allowed(req: &ProfileSnapshotRequest<'_>, reasons: &[String]) -> bool {
    reasons.iter().all(|reason| match reason.as_str() {
        "suppressed" => req.include_suppressed,
        reason if reason.starts_with("provenance:") => req.include_manual_summaries,
        "source:not-default-visible" => {
            req.include_suppressed
                && req.include_sensitive
                && req.include_inactive
                && req.include_deleted
        }
        _ => false,
    })
}

fn reason_allowed(req: &ProfileSnapshotRequest<'_>, reason: &str) -> bool {
    match reason {
        "suppressed" | "status:suppressed" => req.include_suppressed,
        "status:deleted" => req.include_deleted,
        reason if reason.starts_with("sensitivity:") => req.include_sensitive,
        reason if reason.starts_with("status:") => req.include_inactive,
        reason if reason.starts_with("validity:") => req.include_inactive,
        _ => false,
    }
}

fn reason_allowed_by_some_flag(req: &ProfileSnapshotRequest<'_>, reason: &str) -> bool {
    match reason {
        "suppressed" | "status:suppressed" => req.include_suppressed,
        "status:deleted" => req.include_deleted,
        reason if reason.starts_with("sensitivity:") => req.include_sensitive,
        reason if reason.starts_with("status:") => req.include_inactive,
        reason if reason.starts_with("validity:") => req.include_inactive,
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        memory::suppression::{create_suppression, SuppressRequest, SuppressionTarget},
        user_context::{
            claims::{
                create_manual_claim, delete_claim, ManualClaimRequest, UserContextClaimType,
                UserContextSensitivity,
            },
            summary::{refresh_summary, SummaryEditRequest},
        },
    };

    #[test]
    fn profile_snapshot_renders_active_summary_and_claims_in_stable_order() -> Result<()> {
        let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-active");
        let conn = db::open_db()?;
        create_claim(
            &conn,
            "Prefer concise reviews",
            "review",
            UserContextSensitivity::Normal,
        )?;
        create_claim(
            &conn,
            "Use deterministic tests",
            "tests",
            UserContextSensitivity::Normal,
        )?;
        refresh_summary(
            &conn,
            &SummaryRequest {
                owner_scope: None,
                owner_key: None,
                project: "/repo",
            },
        )?;

        let output = render_markdown_profile_snapshot(
            &conn,
            &request("/repo", data_dir.db_path().as_path()),
        )?;

        assert!(output.contains("# remem User Profile Snapshot"));
        assert!(output.contains("Source of truth: SQLite database at"));
        assert!(output.contains("Derived snapshot: editing this Markdown does not mutate"));
        assert!(output.contains("## Active Summary"));
        assert!(output.contains("source_claim_ids: ["));
        assert!(output.contains("active_claim_ids=[1, 2]"));
        let first = output.find("preference:review").unwrap();
        let second = output.find("preference:tests").unwrap();
        assert!(first < second);
        Ok(())
    }

    #[test]
    fn profile_snapshot_excludes_non_default_claims_and_redacts_until_all_audit_flags_allow_text(
    ) -> Result<()> {
        let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-excluded");
        let conn = db::open_db()?;
        create_claim(
            &conn,
            "Normal retained claim",
            "normal",
            UserContextSensitivity::Normal,
        )?;
        create_claim(
            &conn,
            "Personal claim should stay hidden",
            "personal",
            UserContextSensitivity::Personal,
        )?;
        create_claim(
            &conn,
            "Restricted claim should stay hidden",
            "restricted",
            UserContextSensitivity::Restricted,
        )?;
        let future = create_claim(
            &conn,
            "Future claim should stay hidden",
            "future",
            UserContextSensitivity::Normal,
        )?;
        conn.execute(
            "UPDATE user_context_claims SET valid_from_epoch = ?1 WHERE id = ?2",
            rusqlite::params![chrono::Utc::now().timestamp() + 3600, future.id],
        )?;
        let expired = create_claim(
            &conn,
            "Expired claim should stay hidden",
            "expired",
            UserContextSensitivity::Normal,
        )?;
        conn.execute(
            "UPDATE user_context_claims SET valid_to_epoch = ?1 WHERE id = ?2",
            rusqlite::params![chrono::Utc::now().timestamp() - 3600, expired.id],
        )?;
        let sensitive = create_claim(
            &conn,
            "Sensitive suppressed claim",
            "sensitive",
            UserContextSensitivity::Sensitive,
        )?;
        create_suppression(
            &conn,
            &SuppressRequest {
                target: SuppressionTarget {
                    kind: "user_claim".to_string(),
                    id: Some(sensitive.id),
                    value: None,
                },
                reason: Some("audit suppression"),
                actor: Some("test"),
            },
        )?;
        let deleted = create_claim(
            &conn,
            "Deleted claim should stay hidden",
            "deleted",
            UserContextSensitivity::Normal,
        )?;
        delete_claim(&conn, deleted.id)?;

        let default_output = render_markdown_profile_snapshot(
            &conn,
            &request("/repo", data_dir.db_path().as_path()),
        )?;
        assert!(default_output.contains("Normal retained claim"));
        assert!(!default_output.contains("Personal claim should stay hidden"));
        assert!(!default_output.contains("Restricted claim should stay hidden"));
        assert!(!default_output.contains("Future claim should stay hidden"));
        assert!(!default_output.contains("Expired claim should stay hidden"));
        assert!(!default_output.contains("Sensitive suppressed claim"));
        assert!(!default_output.contains("Deleted claim should stay hidden"));

        let db_path = data_dir.db_path();
        let mut suppressed_only = request("/repo", db_path.as_path());
        suppressed_only.include_suppressed = true;
        let output = render_markdown_profile_snapshot(&conn, &suppressed_only)?;
        assert!(output.contains("preference:sensitive - [redacted]"));
        assert!(output.contains("reason: sensitivity:sensitive, suppressed"));
        assert!(!output.contains("Sensitive suppressed claim"));

        let db_path = data_dir.db_path();
        let mut full_audit = request("/repo", db_path.as_path());
        full_audit.include_suppressed = true;
        full_audit.include_sensitive = true;
        full_audit.include_inactive = true;
        full_audit.include_deleted = true;
        let output = render_markdown_profile_snapshot(&conn, &full_audit)?;
        assert!(output.contains("Personal claim should stay hidden"));
        assert!(output.contains("Restricted claim should stay hidden"));
        assert!(output.contains("Future claim should stay hidden"));
        assert!(output.contains("Expired claim should stay hidden"));
        assert!(output.contains("Sensitive suppressed claim"));
        assert!(output.contains("Deleted claim should stay hidden"));
        assert!(output.contains("reason: status:deleted"));
        Ok(())
    }

    #[test]
    fn profile_snapshot_manual_summary_requires_audit_flag() -> Result<()> {
        let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-manual");
        let conn = db::open_db()?;
        crate::user_context::summary::edit_summary(
            &conn,
            &SummaryEditRequest {
                owner_scope: None,
                owner_key: None,
                project: "/repo",
                text: "Manual profile summary text",
            },
        )?;

        let default_output = render_markdown_profile_snapshot(
            &conn,
            &request("/repo", data_dir.db_path().as_path()),
        )?;
        assert!(default_output.contains("No default-eligible active summary text."));
        assert!(!default_output.contains("Manual profile summary text"));
        assert!(default_output.contains("provenance:manual-edit"));

        let db_path = data_dir.db_path();
        let mut audit = request("/repo", db_path.as_path());
        audit.include_manual_summaries = true;
        let output = render_markdown_profile_snapshot(&conn, &audit)?;
        assert!(output.contains("Manual profile summary text"));
        Ok(())
    }

    fn request<'a>(project: &'a str, source_of_truth: &'a Path) -> ProfileSnapshotRequest<'a> {
        ProfileSnapshotRequest {
            project,
            owner_scope: "user",
            owner_key: None,
            source_of_truth,
            include_suppressed: false,
            include_sensitive: false,
            include_inactive: false,
            include_deleted: false,
            include_manual_summaries: false,
        }
    }

    fn create_claim(
        conn: &Connection,
        text: &str,
        key: &str,
        sensitivity: UserContextSensitivity,
    ) -> Result<crate::user_context::claims::UserContextClaim> {
        create_manual_claim(
            conn,
            &ManualClaimRequest {
                text,
                owner_scope: None,
                owner_key: None,
                claim_type: UserContextClaimType::Preference,
                claim_key: Some(key),
                confidence: 1.0,
                sensitivity,
                valid_from_epoch: None,
                valid_to_epoch: None,
            },
        )
    }
}
