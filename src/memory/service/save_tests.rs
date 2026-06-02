use super::{save_memory, SaveMemoryRequest};
use crate::db::{self, test_support::ScopedTestDataDir};

#[test]
fn repeated_lesson_save_reinforces_metadata_and_logs_update() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("lesson-save-reinforces");
    let conn = db::open_db()?;
    let req = SaveMemoryRequest {
        text: "Lesson: keep operation audit without losing lesson reinforcement.".to_string(),
        title: Some("Lesson reinforcement".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("lesson-reinforcement".to_string()),
        memory_type: Some("lesson".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };

    let first = save_memory(&conn, &req)?;
    let second = save_memory(&conn, &req)?;

    assert_eq!(first.id, second.id);
    assert_eq!(second.operation, "update");
    let reinforcement_count: i64 = conn.query_row(
        "SELECT reinforcement_count FROM memory_lessons WHERE memory_id = ?1",
        [first.id],
        |row| row.get(0),
    )?;
    let operations = conn
        .prepare("SELECT operation FROM memory_operation_log ORDER BY id ASC")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(reinforcement_count, 2);
    assert_eq!(operations, vec!["add".to_string(), "update".to_string()]);
    Ok(())
}
