use crate::memory::Memory;

use super::{search::preview_text, show::format_memory_timestamp};

#[test]
fn cli_query_preview_uses_first_line_and_truncates() {
    let memory = Memory {
        id: 1,
        session_id: None,
        project: "proj".to_string(),
        topic_key: None,
        title: "Title".to_string(),
        text: format!("{}\nsecond line", "a".repeat(120)),
        memory_type: "decision".to_string(),
        files: None,
        created_at_epoch: 0,
        updated_at_epoch: 0,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    };

    let preview = preview_text(&memory);
    assert_eq!(preview.len(), 80);
    assert!(preview.chars().all(|ch| ch == 'a'));
}

#[test]
fn cli_query_format_memory_timestamp_handles_invalid_epoch() {
    assert_eq!(format_memory_timestamp(i64::MAX), "");
}
