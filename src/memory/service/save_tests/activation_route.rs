use super::*;

#[test]
fn direct_save_topic_match_ignores_newer_row_outside_repo_owner_route() -> anyhow::Result<()> {
    let _dir = ScopedTestDataDir::new("save-topic-owner-route");
    let conn = db::open_db()?;
    let request = SaveMemoryRequest {
        text: "Initial repo-owned content.".to_string(),
        title: Some("Repo target".to_string()),
        project: Some("proj".to_string()),
        topic_key: Some("owner-route-target".to_string()),
        memory_type: Some("discovery".to_string()),
        scope: Some("project".to_string()),
        local_copy_enabled: Some(false),
        ..SaveMemoryRequest::default()
    };
    let repo_memory = save_memory(&conn, &request)?;
    conn.execute(
        "INSERT INTO memories
         (project, topic_key, title, content, memory_type, created_at_epoch,
          updated_at_epoch, status, scope, source_project, owner_scope,
          owner_key, context_class, source_trust_class)
         VALUES ('proj', 'owner-route-target', 'Tool-owned collision',
                 'Must remain untouched.', 'discovery', 1, 9999999999,
                 'active', 'project', 'proj', 'tool', 'tool:example',
                 'startup_core', 'local_tool_output')",
        [],
    )?;
    let tool_memory = conn.last_insert_rowid();
    let updated = save_memory(
        &conn,
        &SaveMemoryRequest {
            text: "Updated repo-owned content.".to_string(),
            title: Some("Repo target updated".to_string()),
            ..request
        },
    )?;

    assert_eq!(updated.id, repo_memory.id);
    let tool_content: String = conn.query_row(
        "SELECT content FROM memories WHERE id = ?1",
        [tool_memory],
        |row| row.get(0),
    )?;
    assert_eq!(tool_content, "Must remain untouched.");
    Ok(())
}
