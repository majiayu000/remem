use super::*;

#[test]
fn markdown_import_treats_visible_h1_as_title_edit() -> Result<()> {
    let project = "/tmp/remem-markdown-heading-title";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('source-session', ?1, 'heading-topic', 'Original title',
                 'Body content stays body content.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-heading-title");
    export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: false,
            limit: 100,
        },
    )?;
    let path = only_markdown_file(&export_dir)?;
    let raw = std::fs::read_to_string(&path)?;
    std::fs::write(
        &path,
        raw.replace("# Original title", "# Edited visible title"),
    )?;

    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);
    let row: (String, String) = target.query_row(
        "SELECT title, content FROM memories WHERE project = ?1 AND topic_key = 'heading-topic'",
        [project],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(row.0, "Edited visible title");
    assert_eq!(row.1, "Body content stays body content.");

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_treats_crlf_visible_h1_as_title_edit() -> Result<()> {
    let project = "/tmp/remem-markdown-heading-crlf";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('source-session', ?1, 'heading-crlf-topic', 'Original title',
                 'Body content stays body content under CRLF.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-heading-crlf");
    export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: false,
            limit: 100,
        },
    )?;
    let path = only_markdown_file(&export_dir)?;
    let raw = std::fs::read_to_string(&path)?;
    let edited = raw
        .replace("# Original title", "# Edited CRLF visible title")
        .replace('\n', "\r\n");
    std::fs::write(&path, edited)?;

    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);
    let row: (String, String) = target.query_row(
        "SELECT title, content FROM memories
         WHERE project = ?1 AND topic_key = 'heading-crlf-topic'",
        [project],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(row.0, "Edited CRLF visible title");
    assert_eq!(row.1, "Body content stays body content under CRLF.");

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_does_not_move_current_state_backwards_for_older_active_slot() -> Result<()> {
    let project = "/tmp/remem-markdown-current-guard";
    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    let decision = crate::memory::state_key::derive_state_key(
        "decision",
        Some("decision-aaaa1111"),
        "Optimize CJK search",
        "Use FTS5 trigram tokenizer for CJK text search support.",
    )
    .expect("semantic state key should derive");
    target.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', ?1, 'decision', ?2, 'decision slot', 'active',
                 1, 100, 500)",
        rusqlite::params![project, decision.state_key],
    )?;
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          valid_from_epoch, state_key_id)
         VALUES (1, 'current-session', ?1, 'decision-aaaa1111', 'Optimize CJK search',
                 'Use FTS5 trigram tokenizer for CJK text search support.',
                 'decision', NULL, 'current search context',
                 400, 500, 450, 'active', 'main', 'project', 400, 10)",
        [project],
    )?;

    let export_dir = unique_temp_dir("markdown-current-guard");
    std::fs::create_dir_all(&export_dir)?;
    let doc = MarkdownMemoryDocument {
        metadata: MarkdownMemoryMetadata {
            source_id: None,
            project: project.to_string(),
            topic_key: Some("decision-bbbb2222".to_string()),
            title: "Optimize CJK search".to_string(),
            memory_type: "decision".to_string(),
            created_at_epoch: 100,
            updated_at_epoch: 200,
            reference_time_epoch: Some(150),
            ..sample_metadata("active", "project")
        },
        content: "Use FTS5 trigram tokenizer for CJK text search support.".to_string(),
    };
    std::fs::write(
        export_dir.join("older-active.md"),
        render_markdown_memory(&doc),
    )?;

    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);
    let current_memory_id: i64 = target.query_row(
        "SELECT current_memory_id FROM memory_state_keys WHERE id = 10",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(current_memory_id, 1);
    let imported_state_key_id: Option<i64> = target.query_row(
        "SELECT state_key_id FROM memories WHERE topic_key = 'decision-bbbb2222'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(imported_state_key_id, Some(10));

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}
