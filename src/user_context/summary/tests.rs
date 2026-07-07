use super::*;
use crate::memory::suppression::{create_suppression, parse_target, SuppressRequest};
use crate::user_context::claims::{
    create_manual_claim, create_preference_backfill_claim, suppress_claim, ManualClaimRequest,
    PreferenceBackfillClaimRequest, UserContextClaimType, UserContextSensitivity,
};

fn summary_migrated_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

#[test]
fn refresh_compiles_sources_and_filters_unsafe_claims() -> Result<()> {
    let conn = summary_migrated_conn()?;
    let active = create_manual_claim(
        &conn,
        &claim_request("Prefer concise reviews", UserContextSensitivity::Normal),
    )?;
    let personal = create_manual_claim(
        &conn,
        &claim_request(
            "Prefer Chinese for architecture",
            UserContextSensitivity::Personal,
        ),
    )?;
    let suppressed = create_manual_claim(
        &conn,
        &claim_request("Do not include", UserContextSensitivity::Normal),
    )?;
    suppress_claim(&conn, suppressed.id)?;
    create_manual_claim(
        &conn,
        &claim_request("Sensitive identity", UserContextSensitivity::Sensitive),
    )?;
    create_manual_claim(
        &conn,
        &ManualClaimRequest {
            text: "Expired goal",
            valid_to_epoch: Some(1),
            ..claim_request("Expired goal", UserContextSensitivity::Normal)
        },
    )?;
    insert_summary_memory_source(
        &conn,
        10,
        "/repo",
        "Architecture",
        "Use source-backed summaries",
    )?;
    insert_summary_workstream_source(&conn, 20, "/repo", "Ship profile summaries")?;
    insert_summary_session_source(&conn, 30, "/repo", "Reviewed user context design")?;

    let summary = refresh_summary(&conn, &summary_request("/repo"))?;

    assert!(summary.summary_text.contains("Prefer concise reviews"));
    assert!(!summary.summary_text.contains("Prefer Chinese"));
    assert!(summary.summary_text.contains("Use source-backed summaries"));
    assert!(!summary.summary_text.contains("Do not include"));
    assert!(!summary.summary_text.contains("Sensitive identity"));
    assert_eq!(summary.source_claim_ids, vec![active.id]);
    assert_eq!(summary.source_memory_ids, vec![10]);
    assert_eq!(summary.source_activity_refs.len(), 2);

    let sources = load_summary_sources(&conn, &summary_request("/repo"), true)?;
    assert_eq!(sources.included_claims.len(), 1);
    assert!(sources
        .dropped_claims
        .iter()
        .any(|source| source.id == personal.id && source.reason == "sensitivity:personal"));
    assert!(sources
        .dropped_claims
        .iter()
        .any(|source| source.reason == "status:suppressed"));
    assert!(sources
        .dropped_claims
        .iter()
        .any(|source| source.reason == "sensitivity:sensitive"));
    assert!(sources
        .dropped_claims
        .iter()
        .any(|source| source.reason == "expired"));
    Ok(())
}

#[test]
fn refresh_summary_excludes_synthetic_rollup_activity_refs() -> Result<()> {
    let conn = summary_migrated_conn()?;
    insert_summary_session_source(&conn, 30, "/repo", "Reviewed user context design")?;
    conn.execute(
        "INSERT INTO session_summaries
         (id, memory_session_id, project, request, created_at_epoch,
          source_project, target_project, owner_scope, owner_key, session_row_id,
         covered_from_event_id, covered_to_event_id)
         VALUES
         (31, 'rollup-synthetic', '/repo', 'Captured event range 1..3', 11,
          '/repo', '/repo', 'repo', '/repo', 1, 1, 3),
         (32, 'rollup-semantic', '/repo', 'Retire legacy Summary writer', 12,
          '/repo', '/repo', 'repo', '/repo', 2, 4, 6),
         (33, 'rollup-structured-fallback', '/repo', 'Captured event range 7..9', 13,
          '/repo', '/repo', 'repo', '/repo', 3, 7, 9)",
        [],
    )?;
    conn.execute(
        "UPDATE session_summaries
         SET decisions = 'Rollup structured decision'
         WHERE id = 33",
        [],
    )?;

    let sources = load_summary_sources(&conn, &summary_request("/repo"), false)?;
    let labels: Vec<&str> = sources
        .included_activity_refs
        .iter()
        .map(|activity| activity.label.as_str())
        .collect();

    assert!(labels.contains(&"Rollup structured decision"));
    assert!(labels.contains(&"Retire legacy Summary writer"));
    assert!(labels.contains(&"Reviewed user context design"));
    assert!(!labels.contains(&"Captured event range 1..3"));
    assert!(!labels.contains(&"Captured event range 7..9"));
    Ok(())
}

#[test]
fn sources_resolve_stored_ids_after_source_status_changes() -> Result<()> {
    let conn = summary_migrated_conn()?;
    let active = create_manual_claim(
        &conn,
        &claim_request("Keep source provenance", UserContextSensitivity::Normal),
    )?;
    insert_summary_memory_source(&conn, 10, "/repo", "Architecture", "Original memory source")?;
    insert_summary_workstream_source(&conn, 20, "/repo", "Ship profile summaries")?;
    let summary = refresh_summary(&conn, &summary_request("/repo"))?;

    suppress_claim(&conn, active.id)?;
    conn.execute("UPDATE memories SET status = 'stale' WHERE id = 10", [])?;

    let sources = load_summary_sources(&conn, &summary_request("/repo"), true)?;

    assert_eq!(sources.included_claims.len(), 1);
    assert_eq!(sources.included_claims[0].id, active.id);
    assert_eq!(sources.included_claims[0].status, "suppressed");
    assert_eq!(sources.included_memories.len(), 1);
    assert_eq!(sources.included_memories[0].id, 10);
    assert_eq!(sources.included_memories[0].status, "stale");
    assert_eq!(sources.included_activity_refs, summary.source_activity_refs);
    assert!(sources.dropped_claims.is_empty());
    Ok(())
}

#[test]
fn refresh_excludes_policy_suppressed_claims_and_memories() -> Result<()> {
    let conn = summary_migrated_conn()?;
    let visible_claim = create_manual_claim(
        &conn,
        &claim_request("Prefer visible summaries", UserContextSensitivity::Normal),
    )?;
    let hidden_claim = create_manual_claim(
        &conn,
        &claim_request(
            "Do not summarize this claim",
            UserContextSensitivity::Normal,
        ),
    )?;
    create_manual_claim(
        &conn,
        &claim_request(
            "Do not summarize this secret pattern",
            UserContextSensitivity::Normal,
        ),
    )?;
    insert_summary_memory_source(
        &conn,
        10,
        "/repo",
        "Visible memory",
        "Visible memory source",
    )?;
    insert_summary_memory_source(&conn, 11, "/repo", "Hidden memory", "Hidden memory source")?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: parse_target(&format!("claim:{}", hidden_claim.id))?,
            reason: Some("not relevant"),
            actor: Some("test"),
        },
    )?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: parse_target("memory:11")?,
            reason: Some("stale"),
            actor: Some("test"),
        },
    )?;
    create_suppression(
        &conn,
        &SuppressRequest {
            target: parse_target("pattern:secret pattern")?,
            reason: Some("too noisy"),
            actor: Some("test"),
        },
    )?;

    let summary = refresh_summary(&conn, &summary_request("/repo"))?;

    assert_eq!(summary.source_claim_ids, vec![visible_claim.id]);
    assert_eq!(summary.source_memory_ids, vec![10]);
    assert!(summary.summary_text.contains("Prefer visible summaries"));
    assert!(!summary.summary_text.contains("Do not summarize"));
    assert!(!summary.summary_text.contains("secret pattern"));
    assert!(!summary.summary_text.contains("Hidden memory source"));
    Ok(())
}

#[test]
fn refresh_summary_excludes_backfilled_user_preference_memory_source() -> Result<()> {
    let conn = summary_migrated_conn()?;
    insert_summary_user_preference_source(&conn, 12, "Prefer backfilled summary source")?;
    let claim = create_preference_backfill_claim(
        &conn,
        &PreferenceBackfillClaimRequest {
            memory_id: 12,
            text: "Prefer backfilled summary source",
        },
    )?;

    let summary = refresh_summary(&conn, &summary_request("/repo"))?;

    assert_eq!(summary.source_claim_ids, vec![claim.id]);
    assert!(summary.source_memory_ids.is_empty());
    assert!(summary
        .summary_text
        .contains("Prefer backfilled summary source"));
    assert!(!summary.summary_text.contains("[memory:12]"));
    Ok(())
}

#[test]
fn active_summary_is_hidden_after_memory_source_is_backfilled_as_claim() -> Result<()> {
    let conn = summary_migrated_conn()?;
    insert_summary_user_preference_source(&conn, 13, "Prefer stale summary source")?;
    let summary = refresh_summary(&conn, &summary_request("/repo"))?;
    assert_eq!(summary.source_memory_ids, vec![13]);

    create_preference_backfill_claim(
        &conn,
        &PreferenceBackfillClaimRequest {
            memory_id: 13,
            text: "Prefer stale summary source",
        },
    )?;

    assert!(load_active_summary(&conn, &summary_request("/repo"))?.is_none());
    Ok(())
}

#[test]
fn load_active_summary_hides_text_after_policy_suppresses_source() -> Result<()> {
    let conn = summary_migrated_conn()?;
    let claim = create_manual_claim(
        &conn,
        &claim_request("Old profile text", UserContextSensitivity::Normal),
    )?;
    insert_summary_memory_source(&conn, 10, "/repo", "Old memory", "Old memory text")?;
    let summary = refresh_summary(&conn, &summary_request("/repo"))?;
    assert!(summary.summary_text.contains("Old profile text"));
    assert!(load_active_summary(&conn, &summary_request("/repo"))?.is_some());

    create_suppression(
        &conn,
        &SuppressRequest {
            target: parse_target(&format!("claim:{}", claim.id))?,
            reason: Some("do not show"),
            actor: Some("test"),
        },
    )?;

    assert!(load_active_summary(&conn, &summary_request("/repo"))?.is_none());
    let sources = load_summary_sources(&conn, &summary_request("/repo"), false)?;
    assert!(sources.summary.is_none());
    assert!(!sources
        .included_claims
        .iter()
        .any(|item| item.claim_text.contains("Old profile text")));
    let audit_sources = load_summary_sources(&conn, &summary_request("/repo"), true)?;
    assert_eq!(
        audit_sources.summary.as_ref().map(|item| item.id),
        Some(summary.id)
    );
    Ok(())
}

#[test]
fn generator_failure_keeps_previous_active_summary() -> Result<()> {
    let conn = summary_migrated_conn()?;
    create_manual_claim(
        &conn,
        &claim_request("Keep this summary", UserContextSensitivity::Normal),
    )?;
    let first = refresh_summary(&conn, &summary_request("/repo"))?;

    let err =
        refresh_summary_with_generator(&conn, &summary_request("/repo"), |_project, _sources| {
            bail!("model provider unavailable")
        })
        .expect_err("summary generator failure should fail closed");
    assert!(err.to_string().contains("generate profile summary"));
    let current = load_active_summary(&conn, &summary_request("/repo"))?
        .ok_or_else(|| anyhow::anyhow!("previous summary missing"))?;
    assert_eq!(current.id, first.id);
    assert_eq!(current.summary_text, first.summary_text);
    Ok(())
}

#[test]
fn edit_preserves_source_ids_and_supersedes_previous_version() -> Result<()> {
    let conn = summary_migrated_conn()?;
    create_manual_claim(
        &conn,
        &claim_request("Original source", UserContextSensitivity::Normal),
    )?;
    let first = refresh_summary(&conn, &summary_request("/repo"))?;
    let edited = edit_summary(
        &conn,
        &SummaryEditRequest {
            owner_scope: None,
            owner_key: None,
            project: "/repo",
            text: "Edited user-visible summary",
        },
    )?;

    assert_eq!(edited.version, first.version + 1);
    assert_eq!(edited.summary_text, "Edited user-visible summary");
    assert_eq!(edited.source_claim_ids, first.source_claim_ids);
    let old_status: String = conn.query_row(
        "SELECT status FROM user_context_summaries WHERE id = ?1",
        [first.id],
        |row| row.get(0),
    )?;
    assert_eq!(old_status, "superseded");
    Ok(())
}

#[test]
fn source_json_parsers_reject_invalid_shapes_and_source_ids() {
    let err = parse_ids("source_claim_ids_json", "{\"id\":1}")
        .expect_err("source ids must be encoded as an array");
    assert!(err.to_string().contains("JSON integer array"));
    let err = parse_ids("source_claim_ids_json", "[1,\"two\"]")
        .expect_err("source ids must contain integers");
    assert!(err.to_string().contains("JSON integer array"));
    let err = parse_ids("source_claim_ids_json", "[1,0]").expect_err("source ids must be positive");
    assert!(err.to_string().contains("positive integer"));
    let err = parse_activity_refs("{}").expect_err("activity refs must be encoded as an array");
    assert!(err.to_string().contains("JSON array"));
    let err = parse_activity_refs(r#"[{"kind":"workstream","id":0,"label":"x"}]"#)
        .expect_err("activity refs must use positive ids");
    assert!(err.to_string().contains("positive id"));
    let err = parse_activity_refs(r#"[{"kind":"","id":1,"label":"x"}]"#)
        .expect_err("activity refs must include a kind");
    assert!(err.to_string().contains("positive id"));
}

fn summary_request(project: &str) -> SummaryRequest<'_> {
    SummaryRequest {
        owner_scope: None,
        owner_key: None,
        project,
    }
}

fn claim_request(text: &str, sensitivity: UserContextSensitivity) -> ManualClaimRequest<'_> {
    ManualClaimRequest {
        text,
        owner_scope: None,
        owner_key: None,
        claim_type: UserContextClaimType::Preference,
        claim_key: None,
        confidence: 1.0,
        sensitivity,
        valid_from_epoch: None,
        valid_to_epoch: None,
    }
}

fn insert_summary_memory_source(
    conn: &Connection,
    id: i64,
    project: &str,
    title: &str,
    text: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
          target_project, owner_scope, owner_key)
         VALUES (?1, NULL, ?2, NULL, ?3, ?4, 'decision', NULL, 10, 10, 'active',
                 NULL, 'project', ?2, ?2, 'repo', ?2)",
        params![id, project, title, text],
    )?;
    Ok(())
}

fn insert_summary_user_preference_source(conn: &Connection, id: i64, text: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
          target_project, owner_scope, owner_key)
         VALUES (?1, NULL, '/repo', NULL, 'Preference', ?2, 'preference', NULL,
                 10, 10, 'active', NULL, 'global', '/repo', NULL, 'user', 'user:default')",
        params![id, text],
    )?;
    Ok(())
}

fn insert_summary_workstream_source(
    conn: &Connection,
    id: i64,
    project: &str,
    title: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO workstreams
         (id, project, title, status, created_at_epoch, updated_at_epoch,
          source_project, target_project, owner_scope, owner_key)
         VALUES (?1, ?2, ?3, 'active', 10, 10, ?2, ?2, 'repo', ?2)",
        params![id, project, title],
    )?;
    Ok(())
}

fn insert_summary_session_source(
    conn: &Connection,
    id: i64,
    project: &str,
    request: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO session_summaries
         (id, memory_session_id, project, request, created_at_epoch,
          source_project, target_project, owner_scope, owner_key)
         VALUES (?1, 'session-1', ?2, ?3, 10, ?2, ?2, 'repo', ?2)",
        params![id, project, request],
    )?;
    Ok(())
}
