use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::{
    db,
    memory::suppression::{create_suppression, SuppressRequest, SuppressionTarget},
    user_context::{
        claims::{
            create_manual_claim, create_preference_backfill_claim, delete_claim,
            ManualClaimRequest, PreferenceBackfillClaimRequest, UserContextClaim,
            UserContextClaimType, UserContextSensitivity,
        },
        profile_snapshot::{render_markdown_profile_snapshot, ProfileSnapshotRequest},
        summary::{edit_summary, refresh_summary, SummaryEditRequest, SummaryRequest},
    },
};

const SNAPSHOT_CLAIM_LIMIT: usize = 500;

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
        &snapshot_request("/repo", data_dir.db_path().as_path()),
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
        "medical-location-account",
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
    conn.execute(
        "UPDATE user_context_claims
         SET source_refs_json = ?1
         WHERE id = ?2",
        rusqlite::params![
            r#"[{"kind":"manual_cli","path":"/private/medical-location-account.txt"}]"#,
            sensitive.id
        ],
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
        &snapshot_request("/repo", data_dir.db_path().as_path()),
    )?;
    assert!(default_output.contains("Normal retained claim"));
    assert!(!default_output.contains("Personal claim should stay hidden"));
    assert!(!default_output.contains("Restricted claim should stay hidden"));
    assert!(!default_output.contains("Future claim should stay hidden"));
    assert!(!default_output.contains("Expired claim should stay hidden"));
    assert!(!default_output.contains("Sensitive suppressed claim"));
    assert!(!default_output.contains("Deleted claim should stay hidden"));

    let db_path = data_dir.db_path();
    let mut suppressed_only = snapshot_request("/repo", db_path.as_path());
    suppressed_only.include_suppressed = true;
    let output = render_markdown_profile_snapshot(&conn, &suppressed_only)?;
    assert!(output.contains("preference:[redacted] - [redacted]"));
    assert!(output.contains("reason: sensitivity:sensitive, suppressed"));
    assert!(output.contains("source_refs: [redacted]"));
    assert!(!output.contains("Sensitive suppressed claim"));
    assert!(!output.contains("preference:medical-location-account"));
    assert!(!output.contains("medical-location-account.txt"));

    let db_path = data_dir.db_path();
    let mut full_audit = snapshot_request("/repo", db_path.as_path());
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
    assert!(output.contains("medical-location-account.txt"));
    assert!(output.contains("reason: status:deleted"));
    Ok(())
}

#[test]
fn profile_snapshot_manual_summary_requires_audit_flag() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-manual");
    let conn = db::open_db()?;
    edit_summary(
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
        &snapshot_request("/repo", data_dir.db_path().as_path()),
    )?;
    assert!(default_output.contains("No default-eligible active summary text."));
    assert!(!default_output.contains("Manual profile summary text"));
    assert!(!default_output.contains("## Excluded Summary"));
    assert!(!default_output.contains("provenance:manual-edit"));

    let db_path = data_dir.db_path();
    let mut unrelated_audit = snapshot_request("/repo", db_path.as_path());
    unrelated_audit.include_sensitive = true;
    let output = render_markdown_profile_snapshot(&conn, &unrelated_audit)?;
    assert!(output.contains("No default-eligible active summary text."));
    assert!(!output.contains("Manual profile summary text"));
    assert!(!output.contains("## Excluded Summary"));
    assert!(!output.contains("provenance: manual-edit"));

    let mut audit = snapshot_request("/repo", db_path.as_path());
    audit.include_manual_summaries = true;
    let output = render_markdown_profile_snapshot(&conn, &audit)?;
    assert!(output.contains("Manual profile summary text"));
    assert!(output.contains("provenance: manual-edit"));
    assert!(!output.contains("provenance: excluded"));
    Ok(())
}

#[test]
fn profile_snapshot_hides_summary_after_memory_source_is_backfilled() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-backfilled-source");
    let conn = db::open_db()?;
    insert_profile_user_preference_memory(&conn, 100, "Prefer profile backfill dedupe")?;
    let summary = refresh_summary(
        &conn,
        &SummaryRequest {
            owner_scope: None,
            owner_key: None,
            project: "/repo",
        },
    )?;
    assert_eq!(summary.source_memory_ids, vec![100]);
    let claim = create_preference_backfill_claim(
        &conn,
        &PreferenceBackfillClaimRequest {
            memory_id: 100,
            text: "Prefer profile backfill dedupe",
        },
    )?;

    let output = render_markdown_profile_snapshot(
        &conn,
        &snapshot_request("/repo", data_dir.db_path().as_path()),
    )?;

    assert!(output.contains("No default-eligible active summary text."));
    assert!(!output.contains("source_memory_ids: [100]"));
    assert!(output.contains(&format!("[claim:{}]", claim.id)));
    assert!(output.contains("Prefer profile backfill dedupe"));
    Ok(())
}

#[test]
fn profile_snapshot_suppressed_summary_requires_only_suppressed_audit_flag() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-supp-summary");
    let conn = db::open_db()?;
    create_claim(
        &conn,
        "Summary source claim",
        "summary-source",
        UserContextSensitivity::Normal,
    )?;
    let summary = refresh_summary(
        &conn,
        &SummaryRequest {
            owner_scope: None,
            owner_key: None,
            project: "/repo",
        },
    )?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: SuppressionTarget {
                kind: "summary".to_string(),
                id: Some(summary.id),
                value: None,
            },
            reason: Some("summary audit"),
            actor: Some("test"),
        },
    )?;

    let default_output = render_markdown_profile_snapshot(
        &conn,
        &snapshot_request("/repo", data_dir.db_path().as_path()),
    )?;
    assert!(default_output.contains("No default-eligible active summary text."));
    assert!(!default_output.contains("## Excluded Summary"));

    let db_path = data_dir.db_path();
    let mut audit = snapshot_request("/repo", db_path.as_path());
    audit.include_suppressed = true;
    let output = render_markdown_profile_snapshot(&conn, &audit)?;
    let default_count = default_output.matches("Summary source claim").count();
    let audit_count = output.matches("Summary source claim").count();
    assert!(audit_count > default_count);
    assert!(output.contains("reason: suppressed"));
    assert!(output.contains("provenance: source-supported"));
    assert!(!output.contains("source:not-default-visible"));
    Ok(())
}

#[test]
fn profile_snapshot_summary_hidden_by_sensitive_source_requires_only_sensitive_flag() -> Result<()>
{
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-sensitive-summary");
    let conn = db::open_db()?;
    let source = create_claim(
        &conn,
        "Sensitive source-backed summary claim",
        "summary-sensitive-source",
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
    conn.execute(
        "UPDATE user_context_claims
         SET sensitivity = 'sensitive'
         WHERE id = ?1",
        [source.id],
    )?;

    let default_output = render_markdown_profile_snapshot(
        &conn,
        &snapshot_request("/repo", data_dir.db_path().as_path()),
    )?;
    assert!(default_output.contains("No default-eligible active summary text."));
    assert!(!default_output.contains("Sensitive source-backed summary claim"));

    let db_path = data_dir.db_path();
    let mut audit = snapshot_request("/repo", db_path.as_path());
    audit.include_sensitive = true;
    let output = render_markdown_profile_snapshot(&conn, &audit)?;
    assert!(output.contains("Sensitive source-backed summary claim"));
    assert!(output.contains("reason: sensitivity:sensitive"));
    assert!(!output.contains("source:not-default-visible"));
    Ok(())
}

#[test]
fn profile_snapshot_suppressed_summary_still_requires_hidden_source_flags() -> Result<()> {
    let data_dir =
        db::test_support::ScopedTestDataDir::new("profile-snapshot-suppressed-sensitive-summary");
    let conn = db::open_db()?;
    let source = create_claim(
        &conn,
        "Suppressed summary sensitive source claim",
        "summary-suppressed-sensitive-source",
        UserContextSensitivity::Normal,
    )?;
    let summary = refresh_summary(
        &conn,
        &SummaryRequest {
            owner_scope: None,
            owner_key: None,
            project: "/repo",
        },
    )?;
    conn.execute(
        "UPDATE user_context_claims
         SET sensitivity = 'sensitive'
         WHERE id = ?1",
        [source.id],
    )?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: SuppressionTarget {
                kind: "summary".to_string(),
                id: Some(summary.id),
                value: None,
            },
            reason: Some("summary audit"),
            actor: Some("test"),
        },
    )?;

    let db_path = data_dir.db_path();
    let mut suppressed_only = snapshot_request("/repo", db_path.as_path());
    suppressed_only.include_suppressed = true;
    let output = render_markdown_profile_snapshot(&conn, &suppressed_only)?;
    assert!(output.contains("text redacted"));
    assert!(output.contains("reason: suppressed, sensitivity:sensitive"));
    assert!(!output.contains("Suppressed summary sensitive source claim"));
    assert!(!output.contains("source:not-default-visible"));

    let mut full_audit = snapshot_request("/repo", db_path.as_path());
    full_audit.include_suppressed = true;
    full_audit.include_sensitive = true;
    let output = render_markdown_profile_snapshot(&conn, &full_audit)?;
    assert!(output.contains("Suppressed summary sensitive source claim"));
    assert!(output.contains("reason: suppressed, sensitivity:sensitive"));
    Ok(())
}

#[test]
fn profile_snapshot_missing_summary_source_is_visible_to_provenance_audit() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-missing-source");
    let conn = db::open_db()?;
    let source = create_claim(
        &conn,
        "Claim source that will be repaired away",
        "missing-summary-source",
        UserContextSensitivity::Normal,
    )?;
    let summary = refresh_summary(
        &conn,
        &SummaryRequest {
            owner_scope: None,
            owner_key: None,
            project: "/repo",
        },
    )?;
    conn.execute("DELETE FROM user_context_claims WHERE id = ?1", [source.id])?;

    let default_output = render_markdown_profile_snapshot(
        &conn,
        &snapshot_request("/repo", data_dir.db_path().as_path()),
    )?;
    assert!(default_output.contains("No default-eligible active summary text."));
    assert!(!default_output.contains("## Excluded Summary"));
    assert!(!default_output.contains("source:missing"));

    let db_path = data_dir.db_path();
    let mut audit = snapshot_request("/repo", db_path.as_path());
    audit.include_manual_summaries = true;
    let output = render_markdown_profile_snapshot(&conn, &audit)?;
    assert!(output.contains("## Excluded Summary"));
    assert!(output.contains("reason: source:missing"));
    assert!(output.contains(&format!("source_claim_ids: [{}]", source.id)));
    assert!(output.contains(&format!("summary:{}", summary.id)));
    Ok(())
}

#[test]
fn profile_snapshot_claim_limit_applies_after_default_filtering() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-filter-cap");
    let conn = db::open_db()?;
    for index in 0..SNAPSHOT_CLAIM_LIMIT {
        create_claim(
            &conn,
            &format!("Personal prefix claim {index}"),
            &format!("aaa-{index:03}"),
            UserContextSensitivity::Personal,
        )?;
    }
    create_claim(
        &conn,
        "Retained claim after audit-heavy prefix",
        "zzz-retained",
        UserContextSensitivity::Normal,
    )?;

    let output = render_markdown_profile_snapshot(
        &conn,
        &snapshot_request("/repo", data_dir.db_path().as_path()),
    )?;

    assert!(output.contains("Retained claim after audit-heavy prefix"));
    assert!(!output.contains("Personal prefix claim"));
    Ok(())
}

#[test]
fn profile_snapshot_inactive_audit_excludes_unapproved_statuses() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-inactive");
    let conn = db::open_db()?;
    let pending = create_claim(
        &conn,
        "Pending review text must stay hidden",
        "pending",
        UserContextSensitivity::Normal,
    )?;
    let rejected = create_claim(
        &conn,
        "Rejected text must stay hidden",
        "rejected",
        UserContextSensitivity::Normal,
    )?;
    let superseded = create_claim(
        &conn,
        "Superseded text may be audited",
        "superseded",
        UserContextSensitivity::Normal,
    )?;
    conn.execute(
        "UPDATE user_context_claims SET status = 'pending_review' WHERE id = ?1",
        [pending.id],
    )?;
    conn.execute(
        "UPDATE user_context_claims SET status = 'rejected' WHERE id = ?1",
        [rejected.id],
    )?;
    conn.execute(
        "UPDATE user_context_claims SET status = 'superseded' WHERE id = ?1",
        [superseded.id],
    )?;

    let db_path = data_dir.db_path();
    let mut audit = snapshot_request("/repo", db_path.as_path());
    audit.include_inactive = true;
    let output = render_markdown_profile_snapshot(&conn, &audit)?;

    assert!(output.contains("Superseded text may be audited"));
    assert!(output.contains("reason: status:superseded"));
    assert!(!output.contains("Pending review text must stay hidden"));
    assert!(!output.contains("Rejected text must stay hidden"));
    Ok(())
}

#[test]
fn profile_snapshot_scoped_export_includes_default_user_and_repo_claims() -> Result<()> {
    let data_dir = db::test_support::ScopedTestDataDir::new("profile-snapshot-scoped");
    let conn = db::open_db()?;
    create_claim(
        &conn,
        "Default user claim",
        "default-user",
        UserContextSensitivity::Normal,
    )?;
    create_claim_for_owner(
        &conn,
        "Repo claim",
        "repo-claim",
        UserContextSensitivity::Normal,
        Some("repo"),
        Some("/repo"),
    )?;
    create_claim_for_owner(
        &conn,
        "Workspace scoped claim",
        "workspace-claim",
        UserContextSensitivity::Normal,
        Some("workspace"),
        Some("workspace-a"),
    )?;

    let db_path = data_dir.db_path();
    let mut req = snapshot_request("/repo", db_path.as_path());
    req.owner_scope = "workspace";
    req.owner_key = Some("workspace-a");
    let output = render_markdown_profile_snapshot(&conn, &req)?;

    assert!(output.contains("Default user claim"));
    assert!(output.contains("Repo claim"));
    assert!(output.contains("Workspace scoped claim"));
    Ok(())
}

fn snapshot_request<'a>(project: &'a str, source_of_truth: &'a Path) -> ProfileSnapshotRequest<'a> {
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
) -> Result<UserContextClaim> {
    create_claim_for_owner(conn, text, key, sensitivity, None, None)
}

fn create_claim_for_owner(
    conn: &Connection,
    text: &str,
    key: &str,
    sensitivity: UserContextSensitivity,
    owner_scope: Option<&str>,
    owner_key: Option<&str>,
) -> Result<UserContextClaim> {
    create_manual_claim(
        conn,
        &ManualClaimRequest {
            text,
            owner_scope,
            owner_key,
            claim_type: UserContextClaimType::Preference,
            claim_key: Some(key),
            confidence: 1.0,
            sensitivity,
            valid_from_epoch: None,
            valid_to_epoch: None,
        },
    )
}

fn insert_profile_user_preference_memory(conn: &Connection, id: i64, text: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
          target_project, owner_scope, owner_key)
         VALUES (?1, NULL, '/repo', NULL, 'Profile preference', ?2, 'preference', NULL,
                 10, 10, 'active', NULL, 'global', '/repo', NULL, 'user', 'user:default')",
        rusqlite::params![id, text],
    )?;
    Ok(())
}
