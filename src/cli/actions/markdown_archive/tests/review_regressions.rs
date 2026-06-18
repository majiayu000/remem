use super::*;

#[test]
fn markdown_import_source_id_requires_matching_fingerprint() -> Result<()> {
    let project = "/tmp/remem-markdown-source-fingerprint";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (1, 'source-session', ?1, 'source-topic', 'Source decision',
                 'Source markdown content.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-source-fingerprint");
    export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: false,
            limit: 100,
        },
    )?;

    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (1, 'target-session', ?1, 'unrelated-topic', 'Target decision',
                 'Unrelated target content must not be overwritten.',
                 'decision', NULL, 'old search context',
                 900, 950, 925, 'active', 'main', 'project')",
        [project],
    )?;

    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);
    assert_eq!(stats.updated, 0);
    let unrelated: String =
        target.query_row("SELECT content FROM memories WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    assert_eq!(
        unrelated,
        "Unrelated target content must not be overwritten."
    );
    let count: i64 = target.query_row(
        "SELECT COUNT(*) FROM memories WHERE project = ?1",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(count, 2);

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_topic_fallback_prefers_active_memory() -> Result<()> {
    let project = "/tmp/remem-markdown-active-first";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('source-session', ?1, 'shared-topic', 'Imported decision',
                 'Imported content should update the active row.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-active-first");
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
    let mut doc = parse_markdown_memory(&std::fs::read_to_string(&path)?)?;
    doc.metadata.source_id = None;
    std::fs::write(&path, render_markdown_memory(&doc))?;

    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (1, 'old-session', ?1, 'shared-topic', 'Old decision',
                 'Stale content should stay stale.',
                 'decision', NULL, 'old search context',
                 50, 90, 70, 'stale', 'main', 'project')",
        [project],
    )?;
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (2, 'active-session', ?1, 'shared-topic', 'Active decision',
                 'Active content should be replaced.',
                 'decision', NULL, 'old search context',
                 60, 120, 80, 'active', 'main', 'project')",
        [project],
    )?;

    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 0);
    assert_eq!(stats.updated, 1);
    let rows: (String, String, String) = target.query_row(
        "SELECT stale.content, active.content, active.session_id
         FROM memories stale
         JOIN memories active ON active.id = 2
         WHERE stale.id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(rows.0, "Stale content should stay stale.");
    assert_eq!(rows.1, "Imported content should update the active row.");
    assert_eq!(rows.2, "active-session");

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_update_preserves_existing_session_id() -> Result<()> {
    let project = "/tmp/remem-markdown-session";
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('anchored-session', ?1, 'session-topic', 'Session decision',
                 'Original anchored content.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-session");
    export_markdown_archive(
        &conn,
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
        raw.replace("Original anchored content.", "Edited anchored content."),
    )?;

    let stats = import_markdown_archive(&conn, &export_dir, false)?;
    assert_eq!(stats.updated, 1);
    let session_id: Option<String> = conn.query_row(
        "SELECT session_id FROM memories WHERE project = ?1 AND topic_key = 'session-topic'",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(session_id.as_deref(), Some("anchored-session"));

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_preserves_inactive_state_key_history() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("markdown-inactive-state-history");
    let project = "/tmp/remem-markdown-inactive-history";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          valid_from_epoch, valid_to_epoch)
         VALUES ('source-session', ?1, 'inactive-topic', 'Inactive decision',
                 'Inactive content should remain historically addressable.',
                 'decision', NULL, 'old search context',
                 100, 300, 150, 'stale', 'main', 'project', 100, 250)",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-inactive-history");
    export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: true,
            limit: 100,
        },
    )?;

    let target = crate::db::open_db()?;
    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);
    let row: (Option<i64>, Option<i64>) = target.query_row(
        "SELECT m.state_key_id, sk.current_memory_id
         FROM memories m
         LEFT JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.project = ?1 AND m.topic_key = 'inactive-topic'",
        [project],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert!(row.0.is_some());
    assert!(row.1.is_none());

    let historical = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "inactive-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch: Some(200),
            ..Default::default()
        },
    )?;
    assert_eq!(historical.status, "current");
    assert!(historical.current.is_some());
    let current = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "inactive-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(current.status, "no_current");
    assert!(current.current.is_none());

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_extends_existing_state_key_to_older_inactive_history() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("markdown-mixed-state-history");
    let project = "/tmp/remem-markdown-mixed-history";
    let target = crate::db::open_db()?;
    target.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', ?1, 'decision', 'mixed-topic', 'mixed topic', 'active',
                 NULL, 300, 320)",
        [project],
    )?;
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          valid_from_epoch, valid_to_epoch)
         VALUES (1, 'history-session', ?1, 'mixed-topic', 'Mixed historical decision',
                 'Old target history.',
                 'decision', NULL, 'old search context',
                 100, 180, 150, 'stale', 'main', 'project', 100, 250)",
        [project],
    )?;
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          valid_from_epoch, state_key_id)
         VALUES (2, 'current-session', ?1, 'mixed-topic', 'Mixed current decision',
                 'Current content.',
                 'decision', NULL, 'current search context',
                 300, 320, 300, 'active', 'main', 'project', 300, 10)",
        [project],
    )?;
    target.execute(
        "UPDATE memory_state_keys SET current_memory_id = 2 WHERE id = 10",
        [],
    )?;

    let export_dir = unique_temp_dir("markdown-export-mixed-history");
    std::fs::create_dir_all(&export_dir)?;
    let doc = MarkdownMemoryDocument {
        metadata: MarkdownMemoryMetadata {
            source_id: Some(1),
            project: project.to_string(),
            topic_key: Some("mixed-topic".to_string()),
            title: "Mixed historical decision".to_string(),
            memory_type: "decision".to_string(),
            created_at_epoch: 100,
            updated_at_epoch: 180,
            reference_time_epoch: Some(150),
            status: "stale".to_string(),
            branch: Some("main".to_string()),
            valid_from_epoch: Some(100),
            valid_to_epoch: Some(250),
            ..sample_metadata("stale", "project")
        },
        content: "Imported historical content.".to_string(),
    };
    std::fs::write(
        export_dir.join("mixed-history.md"),
        render_markdown_memory(&doc),
    )?;

    let before = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "mixed-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch: Some(200),
            ..Default::default()
        },
    )?;
    assert_eq!(before.status, "not_found");

    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 0);
    assert_eq!(stats.updated, 1);
    let state_key: (i64, Option<i64>) = target.query_row(
        "SELECT created_at_epoch, current_memory_id
         FROM memory_state_keys WHERE id = 10",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(state_key, (100, Some(2)));

    let historical = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "mixed-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch: Some(200),
            ..Default::default()
        },
    )?;
    assert_eq!(historical.status, "current");
    assert_eq!(historical.current.as_ref().map(|memory| memory.id), Some(1));

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

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
