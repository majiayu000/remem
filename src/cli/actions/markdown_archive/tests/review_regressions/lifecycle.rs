use super::*;

#[test]
fn markdown_round_trip_preserves_memory_edges_with_remapped_memory_ids() -> Result<()> {
    let project = "/tmp/remem-markdown-edges";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    create_memory_facts_schema(&source)?;
    let decision = crate::memory::state_key::derive_state_key(
        "decision",
        Some("decision-11111111"),
        "Optimize CJK search",
        "Use FTS5 trigram tokenizer for CJK text search support.",
    )
    .expect("semantic state key should derive");
    source.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', ?1, 'decision', ?2, 'edge topic', 'active',
                 2, 100, 300)",
        rusqlite::params![project, decision.state_key],
    )?;
    source.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          valid_from_epoch, valid_to_epoch, state_key_id)
         VALUES (1, 'old-session', ?1, 'decision-11111111', 'Optimize CJK search',
                 'Use FTS5 trigram tokenizer for CJK text search support.',
                 'decision', NULL, 'old search context',
                 100, 150, 120, 'stale', 'main', 'project', 100, 200, 10)",
        [project],
    )?;
    source.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          valid_from_epoch, state_key_id)
         VALUES (2, 'current-session', ?1, 'decision-22222222', 'Optimize CJK search',
                 'Use FTS5 trigram tokenizer for CJK text search support.',
                 'decision', NULL, 'current search context',
                 200, 300, 220, 'active', 'main', 'project', 200, 10)",
        [project],
    )?;
    source.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, evidence_event_ids,
          confidence, reason, created_at_epoch)
         VALUES ('supersedes', 1, 2, 10, '[31,32]', 0.82,
                 'current replaces old edge decision', 310)",
        [],
    )?;

    let export_dir = unique_temp_dir("markdown-export-edges");
    export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: true,
            limit: 100,
        },
    )?;

    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    create_memory_facts_schema(&target)?;
    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 2);
    let edge: (
        String,
        i64,
        i64,
        Option<i64>,
        Vec<i64>,
        Option<i64>,
        Option<i64>,
        String,
    ) = target.query_row(
        "SELECT e.edge_type, e.from_memory_id, e.to_memory_id, e.state_key_id,
                    e.evidence_event_ids, e.source_candidate_id, e.source_operation_id, e.reason
             FROM memory_edges e",
        [],
        |row| {
            let event_json: Option<String> = row.get(4)?;
            let events = event_json
                .map(|json| serde_json::from_str::<Vec<i64>>(&json).unwrap())
                .unwrap_or_default();
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                events,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        },
    )?;
    assert_eq!(edge.0, "supersedes");
    assert_eq!(edge.1, 1);
    assert_eq!(edge.2, 2);
    assert!(edge.3.is_some());
    assert!(edge.4.is_empty());
    assert!(edge.5.is_none());
    assert!(edge.6.is_none());
    assert_eq!(edge.7, "current replaces old edge decision");

    let state = current_state(
        &target,
        &CurrentStateRequest {
            state_key: decision.state_key,
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            include_history: true,
            ..Default::default()
        },
    )?;
    assert_eq!(state.status, "current");
    assert_eq!(state.history.len(), 1);
    assert_eq!(state.history[0].relation.as_deref(), Some("supersedes"));
    assert_eq!(
        state.history[0].reason.as_deref(),
        Some("current replaces old edge decision")
    );

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
fn markdown_import_topic_fallback_requires_owner_and_type_match() -> Result<()> {
    let project = "/tmp/remem-markdown-owner-type";
    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          owner_scope, owner_key)
         VALUES (1, 'workstream-session', ?1, 'shared-topic', 'Workstream decision',
                 'Workstream-owned content must not be overwritten.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project',
                 'workstream', 'workstream:alpha')",
        [project],
    )?;
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (2, 'procedure-session', ?1, 'shared-topic', 'Procedure memory',
                 'Different-type content must not be overwritten.',
                 'procedure', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;

    let export_dir = unique_temp_dir("markdown-export-owner-type");
    std::fs::create_dir_all(&export_dir)?;
    let doc = MarkdownMemoryDocument {
        metadata: MarkdownMemoryMetadata {
            source_id: None,
            project: project.to_string(),
            topic_key: Some("shared-topic".to_string()),
            title: "Repo decision".to_string(),
            memory_type: "decision".to_string(),
            scope: "project".to_string(),
            ..sample_metadata("active", "project")
        },
        content: "Repo-owned import content.".to_string(),
    };
    std::fs::write(
        export_dir.join("owner-type.md"),
        render_markdown_memory(&doc),
    )?;

    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);
    assert_eq!(stats.updated, 0);
    let rows: (i64, String, String) = target.query_row(
        "SELECT COUNT(*),
                (SELECT content FROM memories WHERE id = 1),
                (SELECT content FROM memories WHERE id = 2)
         FROM memories
         WHERE project = ?1 AND topic_key = 'shared-topic'",
        [project],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(rows.0, 3);
    assert_eq!(rows.1, "Workstream-owned content must not be overwritten.");
    assert_eq!(rows.2, "Different-type content must not be overwritten.");

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

    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    create_memory_facts_schema(&target)?;
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
    let project = "/tmp/remem-markdown-mixed-history";
    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    create_memory_facts_schema(&target)?;
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
            source_content_hash: Some(import_lookup::markdown_source_content_hash(
                "Mixed historical decision",
                "Old target history.",
                "decision",
                Some("mixed-topic"),
            )),
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
