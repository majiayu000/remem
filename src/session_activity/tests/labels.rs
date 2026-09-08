use super::*;

#[test]
fn activity_labels_require_trusted_identity_and_filter_created_epoch() -> Result<()> {
    let conn = setup()?;
    let created = 1_735_660_800;
    insert_message(&conn, 1, "user", "request", created)?;
    insert_message(&conn, 2, "assistant", "result", created + 86_400)?;
    let unclassified =
        list_activity_sessions(&conn, None, Some("abstain"), None, None, None, 10, |_| true)?;
    assert_eq!(unclassified.data[0].session_row_id, Some(1));
    assert!(!unclassified.data[0].override_available);
    conn.execute("INSERT INTO session_summaries (memory_session_id, project, session_row_id, created_at_epoch, session_intent, session_topic, session_intent_source) VALUES ('session-1', '/repo', 1, ?1, 'fix', 'Cursor paging', 'summary')", [created])?;
    let page = list_activity_sessions(
        &conn,
        None,
        Some("fix"),
        Some(created),
        Some(created + 1),
        None,
        10,
        |_| true,
    )?;
    assert_eq!(page.data.len(), 1);
    let item = &page.data[0];
    assert_eq!(item.session_row_id, Some(1));
    assert!(item.override_available);
    assert_eq!(
        item.display_label.as_deref(),
        Some("0101｜fix｜Cursor paging")
    );
    assert!(
        list_activity_sessions(&conn, None, None, None, Some(created), None, 10, |_| true)?
            .data
            .is_empty()
    );
    assert!(
        list_activity_sessions(&conn, None, Some("abstain"), None, None, None, 10, |_| true)?
            .data
            .is_empty()
    );
    conn.execute("UPDATE raw_messages SET source_root = 'remote'", [])?;
    let page =
        list_activity_sessions(&conn, None, Some("abstain"), None, None, None, 10, |_| true)?;
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].session_row_id, None);
    assert!(!page.data[0].override_available);
    assert_eq!(page.data[0].mmdd.as_deref(), Some("0101"));
    assert_eq!(page.data[0].session_intent, None);
    Ok(())
}

#[test]
fn activity_filter_cursor_binds_each_filter_and_advances_empty_pages() -> Result<()> {
    let conn = setup()?;
    for id in 1..=70 {
        insert_message(&conn, id, "assistant", "result", id)?;
    }
    let page = list_activity_sessions(
        &conn,
        Some("/repo"),
        Some("fix"),
        Some(1),
        Some(100),
        None,
        1,
        |_| true,
    )?;
    assert!(page.data.is_empty());
    assert!(page.has_more);
    let cursor = page.next_cursor.as_deref();
    for (project, intent, since, until) in [
        (None, Some("fix"), Some(1), Some(100)),
        (Some("/repo"), Some("doc"), Some(1), Some(100)),
        (Some("/repo"), Some("fix"), Some(2), Some(100)),
        (Some("/repo"), Some("fix"), Some(1), Some(101)),
    ] {
        assert!(
            list_activity_sessions(&conn, project, intent, since, until, cursor, 1, |_| true)
                .is_err()
        );
    }
    let last = list_activity_sessions(
        &conn,
        Some("/repo"),
        Some("fix"),
        Some(1),
        Some(100),
        cursor,
        1,
        |_| true,
    )?;
    assert!(!last.has_more);
    assert!(last.data.is_empty());
    Ok(())
}

#[test]
fn activity_label_redacts_before_render_and_suppresses_before_filter() -> Result<()> {
    let conn = setup()?;
    insert_message(&conn, 1, "user", "request", 100)?;
    conn.execute("UPDATE raw_session_identities SET transcript_path = '/custom/transcript.jsonl', host = 'codex-cli'", [])?;
    let topic = "repair token=sk-abcdefghijklmnopqrstuvwxyz123456 alice@example.com";
    conn.execute("INSERT INTO session_summaries (memory_session_id, project, session_row_id, created_at_epoch, session_intent, session_topic, session_intent_source) VALUES ('session-1','/repo',1,100,'fix',?1,'summary')", [topic])?;
    let page = list_activity_sessions(&conn, None, None, None, None, None, 10, |_| true)?;
    assert_eq!(page.data[0].session_row_id, Some(1));
    let serialized = serde_json::to_string(&page)?;
    assert!(!serialized.contains("sk-abcdefghijklmnopqrstuvwxyz123456"));
    let hidden =
        list_activity_sessions(&conn, None, Some("fix"), None, None, None, 10, |visible| {
            !visible
                .iter()
                .any(|value| value.contains("alice@example.com"))
        })?;
    assert!(hidden.data.is_empty());
    assert!(!serde_json::to_string(&hidden)?.contains("alice@example.com"));
    conn.execute("UPDATE session_summaries SET session_topic = NULL", [])?;
    let abstain =
        list_activity_sessions(&conn, None, Some("abstain"), None, None, None, 10, |_| true)?;
    assert_eq!(abstain.data.len(), 1);
    assert_eq!(abstain.data[0].session_intent.as_deref(), Some("fix"));
    assert!(abstain.data[0].display_label.is_none());
    conn.execute("UPDATE raw_session_identities SET host = NULL, transcript_path = '/home/test/.codex/sessions/looks-codex.jsonl'", [])?;
    let unknown = list_activity_sessions(&conn, None, None, None, None, None, 10, |_| true)?;
    assert_eq!(unknown.data[0].session_row_id, None);
    assert!(!unknown.data[0].override_available);
    Ok(())
}
