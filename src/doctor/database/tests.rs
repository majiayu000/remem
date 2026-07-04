use super::*;

fn setup_conn() -> anyhow::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn record_capture(conn: &Connection) -> anyhow::Result<()> {
    crate::db::record_captured_event(
        conn,
        &crate::db::CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-doctor",
            project: "/tmp/remem-doctor",
            cwd: Some("/tmp/remem-doctor"),
            event_type: "message",
            role: Some("user"),
            tool_name: None,
            content: "captured event",
            task_kind: None,
        },
    )?;
    Ok(())
}

#[test]
fn declared_empty_surfaces_warns_when_source_data_exists_without_rows() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    crate::memory::insert_memory(
        &conn,
        Some("sess"),
        "/tmp/remem",
        None,
        "decision",
        "source memory",
        "decision",
        None,
    )?;
    record_capture(&conn)?;

    let check = check_declared_empty_surfaces(Some(&conn));

    assert!(matches!(check.status, Status::Warn));
    assert!(check.detail.contains("memory_facts=0"));
    assert!(check.detail.contains("graph_edges=0"));
    assert!(check.detail.contains("rule_candidates=0"));
    Ok(())
}

#[test]
fn promotion_funnel_warns_when_observations_do_not_create_candidates() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    record_capture(&conn)?;
    crate::db::insert_observation(
        &conn,
        "sess-doctor",
        "/tmp/remem-doctor",
        "feature",
        Some("observed feature"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        1,
    )?;

    let check = check_promotion_funnel(Some(&conn));

    assert!(matches!(check.status, Status::Warn));
    assert!(check.detail.contains("captured_events=1 -> observations=1"));
    assert!(check
        .detail
        .contains("promotion is not producing candidates"));
    Ok(())
}

#[test]
fn promotion_funnel_warns_when_all_candidates_stay_pending_review() -> anyhow::Result<()> {
    let conn = setup_conn()?;
    record_capture(&conn)?;
    crate::db::insert_observation(
        &conn,
        "sess-doctor",
        "/tmp/remem-doctor",
        "feature",
        Some("observed feature"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        1,
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
         (scope, memory_type, topic_key, text, evidence_event_ids, confidence,
          risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES ('project', 'decision', 'doctor-promotion',
                 'Doctor should flag stalled candidate review.', '[1]', 0.9,
                 'low', 'pending_review', 1, 1)",
        [],
    )?;

    let check = check_promotion_funnel(Some(&conn));

    assert!(matches!(check.status, Status::Warn));
    assert!(check.detail.contains("candidates=1"));
    assert!(check.detail.contains("candidates are all pending review"));
    Ok(())
}
