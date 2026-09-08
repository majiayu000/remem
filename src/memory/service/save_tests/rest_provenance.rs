use crate::db::{self, test_support::ScopedTestDataDir};
use crate::memory::service::{
    save_memory_from_with_reference_time, SaveMemoryCaller, SaveMemoryRequest,
};

#[test]
fn rest_direct_save_with_arbitrary_files_is_not_current_context_eligible() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-rest-files-not-current");
    let conn = db::open_db()?;
    let saved = save_memory_from_with_reference_time(
        &conn,
        &SaveMemoryRequest {
            text: "Use Remem as the six-week trial mainline.".to_string(),
            title: Some("Six-week trial direction".to_string()),
            project: Some("/repo".to_string()),
            topic_key: Some("open-source-six-week-direction".to_string()),
            memory_type: Some("decision".to_string()),
            files: Some(vec!["not-a-real-source".to_string()]),
            local_copy_enabled: Some(false),
            claim_source: Some("case-2026-09-05-manual-save-new".to_string()),
            ..SaveMemoryRequest::default()
        },
        None,
        SaveMemoryCaller::RestAgent,
    )?;

    let now = chrono::Utc::now().timestamp();
    let visibility = crate::truth::classify_memory(&conn, saved.id, now)?;
    let trust: String = conn.query_row(
        "SELECT source_trust_class FROM memories WHERE id = ?1",
        [saved.id],
        |row| row.get(0),
    )?;

    assert_eq!(saved.claim_status, "saved");
    assert!(saved.claim_id.is_some());
    assert_eq!(trust, "external_content");
    assert!(
        !visibility.current_context_eligible,
        "REST save receipts and unverified file paths are not G2 writer proof, got {:?}",
        visibility.reason
    );
    assert_eq!(
        visibility.reason,
        crate::truth::MemoryVisibilityReason::ProvenanceMissing
    );
    Ok(())
}
