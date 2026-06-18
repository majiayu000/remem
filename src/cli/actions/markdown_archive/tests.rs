use super::*;
use crate::db::test_support::ScopedTestDataDir;
use crate::memory::current_state::{current_state, CurrentStateRequest};
use crate::memory::service::{search_memories, SearchRequest};
use crate::memory::types::tests_helper::setup_memory_schema;
use std::path::{Path, PathBuf};

fn unique_temp_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("remem-{label}-{}-{nanos}", std::process::id()))
}

fn only_markdown_file(dir: &Path) -> Result<PathBuf> {
    let mut files = markdown_files(dir)?;
    assert_eq!(files.len(), 1, "{files:?}");
    Ok(files.remove(0))
}

fn sample_metadata(status: &str, scope: &str) -> MarkdownMemoryMetadata {
    MarkdownMemoryMetadata {
        remem_export_version: EXPORT_VERSION,
        source_id: Some(7),
        project: "/tmp/remem-markdown-test".to_string(),
        topic_key: Some("sample".to_string()),
        title: "Sample decision".to_string(),
        memory_type: "decision".to_string(),
        files: None,
        created_at_epoch: 100,
        updated_at_epoch: 200,
        reference_time_epoch: Some(100),
        status: status.to_string(),
        branch: None,
        scope: scope.to_string(),
        source_project: None,
        target_project: None,
        owner_scope: None,
        owner_key: None,
        topic_domain: None,
        routing_confidence: None,
        routing_reason: None,
        context_class: None,
        expires_at_epoch: None,
        valid_from_epoch: None,
        valid_to_epoch: None,
        evidence_event_ids: None,
        source_candidate_id: None,
        lesson: None,
    }
}

#[test]
fn markdown_export_import_round_trip_rebuilds_searchable_memory() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("markdown-archive-round-trip");
    let project = "/tmp/remem-markdown-round-trip";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('s1', ?1, 'round-trip-topic', 'Round trip decision',
                 'Markdown mirror content should survive a human-editable export.',
                 'decision', '[\"src/lib.rs\"]', 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-round-trip");
    let stats = export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: false,
            limit: 100,
        },
    )?;
    assert_eq!(stats.exported, 1);

    let target = crate::db::open_db()?;
    let import_stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(import_stats.imported, 1);
    assert_eq!(import_stats.updated, 0);
    assert_eq!(import_stats.skipped, 0);

    let results = search_memories(
        &target,
        &SearchRequest {
            query: Some("human-editable export".to_string()),
            project: Some(project.to_string()),
            memory_type: None,
            limit: 10,
            offset: 0,
            include_stale: false,
            branch: Some("main".to_string()),
            multi_hop: false,
            explain: false,
        },
    )?;
    assert_eq!(results.memories.len(), 1);
    assert_eq!(
        results.memories[0].topic_key.as_deref(),
        Some("round-trip-topic")
    );
    let row: (
        i64,
        String,
        String,
        String,
        Option<String>,
        String,
        Option<i64>,
    ) = target.query_row(
        "SELECT reference_time_epoch, search_context, owner_scope, owner_key,
                target_project, context_class, state_key_id
         FROM memories WHERE topic_key = 'round-trip-topic'",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    assert_eq!(row.0, 150);
    assert!(row.1.contains("type: decision"));
    assert!(row.1.contains("topic: round trip topic"));
    assert!(row.1.contains("src/lib.rs"));
    assert_eq!(row.2, "repo");
    assert_eq!(row.3, project);
    assert_eq!(row.4.as_deref(), Some(project));
    assert_eq!(row.5, "startup_core");
    assert!(row.6.is_some());

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_updates_existing_memory_and_current_state() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("markdown-archive-update");
    let project = "/tmp/remem-markdown-update";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('s1', ?1, 'update-topic', 'Markdown update decision',
                 'Original markdown mirror content.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-update");
    export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: false,
            limit: 100,
        },
    )?;

    let target = crate::db::open_db()?;
    let first = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(first.imported, 1);
    assert_eq!(first.updated, 0);
    let original_as_of = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "update-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch: Some(225),
            ..Default::default()
        },
    )?;
    assert_eq!(original_as_of.status, "current");

    let path = only_markdown_file(&export_dir)?;
    let raw = std::fs::read_to_string(&path)?;
    std::fs::write(
        &path,
        raw.replace(
            "Original markdown mirror content.",
            "Edited markdown mirror content should be searchable after reindex.",
        ),
    )?;
    let second = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(second.imported, 0);
    assert_eq!(second.updated, 1);

    let count: i64 = target.query_row(
        "SELECT COUNT(*) FROM memories WHERE project = ?1 AND topic_key = 'update-topic'",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(count, 1);
    let edited: String = target.query_row(
        "SELECT content FROM memories WHERE project = ?1 AND topic_key = 'update-topic'",
        [project],
        |row| row.get(0),
    )?;
    assert!(edited.contains("Edited markdown mirror content"));
    let updated_at: i64 = target.query_row(
        "SELECT updated_at_epoch FROM memories WHERE project = ?1 AND topic_key = 'update-topic'",
        [project],
        |row| row.get(0),
    )?;
    assert!(updated_at > 200, "{updated_at}");

    let state = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "update-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(state.status, "current");
    assert!(state
        .current
        .as_ref()
        .is_some_and(|memory| memory.text.contains("Edited markdown mirror content")));
    let before_edit = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "update-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch: Some(225),
            ..Default::default()
        },
    )?;
    assert_eq!(before_edit.status, "no_current");
    assert!(before_edit.current.is_none());

    let mut doc = parse_markdown_memory(&std::fs::read_to_string(&path)?)?;
    doc.metadata.topic_key = Some("renamed-topic".to_string());
    std::fs::write(&path, render_markdown_memory(&doc))?;
    let rename = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(rename.imported, 0);
    assert_eq!(rename.updated, 1);

    let renamed_updated_at: i64 = target.query_row(
        "SELECT updated_at_epoch FROM memories WHERE project = ?1 AND topic_key = 'renamed-topic'",
        [project],
        |row| row.get(0),
    )?;
    assert!(
        renamed_updated_at > updated_at,
        "{renamed_updated_at} <= {updated_at}"
    );
    let renamed_before = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "renamed-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch: Some(renamed_updated_at - 1),
            ..Default::default()
        },
    )?;
    assert_ne!(renamed_before.status, "current");
    assert!(renamed_before.current.is_none());
    let renamed_current = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "renamed-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(renamed_current.status, "current");
    let old_key = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "update-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(old_key.status, "no_current");
    assert!(old_key.current.is_none());

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_update_clears_obsolete_current_state_links() -> Result<()> {
    let _data_dir = ScopedTestDataDir::new("markdown-archive-current-state-clear");
    let project = "/tmp/remem-markdown-current-state-clear";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('s1', ?1, 'same-topic', 'Same topic decision',
                 'Decision: same-topic starts as a decision memory.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-state-clear");
    export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: false,
            limit: 100,
        },
    )?;

    let target = crate::db::open_db()?;
    let first = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(first.imported, 1);

    let path = only_markdown_file(&export_dir)?;
    let mut doc = parse_markdown_memory(&std::fs::read_to_string(&path)?)?;
    doc.metadata.memory_type = "bugfix".to_string();
    doc.metadata.title = "Same topic bugfix".to_string();
    doc.metadata.updated_at_epoch = 250;
    doc.content = "Bugfix: same-topic now describes a bugfix memory.".to_string();
    std::fs::write(&path, render_markdown_memory(&doc))?;
    let second = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(second.updated, 1);

    let old_type = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "same-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("decision".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(old_type.status, "no_current");
    assert!(old_type.current.is_none());

    let new_type = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "same-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("bugfix".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(new_type.status, "current");
    assert_eq!(
        new_type
            .current
            .as_ref()
            .map(|memory| memory.memory_type.as_str()),
        Some("bugfix")
    );

    doc.metadata.topic_key = None;
    doc.metadata.memory_type = "session_activity".to_string();
    doc.metadata.title = "Brief activity".to_string();
    doc.metadata.updated_at_epoch = 275;
    doc.content = "Brief.".to_string();
    std::fs::write(&path, render_markdown_memory(&doc))?;
    let no_key_update = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(no_key_update.updated, 1);
    let old_bugfix = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "same-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("bugfix".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(old_bugfix.status, "no_current");
    let stored_state_key_id: Option<i64> = target.query_row(
        "SELECT state_key_id FROM memories WHERE project = ?1 AND title = 'Brief activity'",
        [project],
        |row| row.get(0),
    )?;
    assert!(stored_state_key_id.is_none());

    doc.metadata.status = "stale".to_string();
    doc.metadata.updated_at_epoch = 300;
    std::fs::write(&path, render_markdown_memory(&doc))?;
    let third = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(third.updated, 1);

    let inactive = current_state(
        &target,
        &CurrentStateRequest {
            state_key: "same-topic".to_string(),
            project: Some(project.to_string()),
            memory_type: Some("bugfix".to_string()),
            ..Default::default()
        },
    )?;
    assert_eq!(inactive.status, "no_current");
    assert!(inactive.current.is_none());

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_updates_exported_null_topic_memory_by_source_id() -> Result<()> {
    let project = "/tmp/remem-markdown-null-topic";
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('s1', ?1, NULL, 'Null topic decision',
                 'Original null-topic markdown content.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-null-topic");
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
        raw.replace(
            "Original null-topic markdown content.",
            "Edited null-topic markdown content.",
        ),
    )?;

    let stats = import_markdown_archive(&conn, &export_dir, false)?;
    assert_eq!(stats.imported, 0);
    assert_eq!(stats.updated, 1);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE project = ?1",
        [project],
        |row| row.get(0),
    )?;
    assert_eq!(count, 1);
    let row: (Option<String>, String) = conn.query_row(
        "SELECT topic_key, content FROM memories WHERE project = ?1",
        [project],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert!(row.0.is_none());
    assert!(row.1.contains("Edited null-topic markdown content"));

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_round_trip_preserves_extended_memory_metadata() -> Result<()> {
    let project = "/tmp/remem-markdown-metadata";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute_batch(
        "ALTER TABLE memories ADD COLUMN evidence_event_ids TEXT;
         ALTER TABLE memories ADD COLUMN source_candidate_id INTEGER;",
    )?;
    source.execute("INSERT INTO memory_candidates(id) VALUES (42)", [])?;
    source.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          source_project, target_project, owner_scope, owner_key, topic_domain,
          routing_confidence, routing_reason, context_class, expires_at_epoch, valid_from_epoch,
          valid_to_epoch, evidence_event_ids, source_candidate_id)
         VALUES (7, 's1', ?1, 'lesson-topic', 'Lesson metadata',
                 'Lesson: preserve extended markdown metadata.',
                 'lesson', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project',
                 '/source', '/target', 'workstream', 'ws:1', 'imports',
                 0.91, 'manual routing', 'task_context', 400, 125, 350, '[11,12]', 42)",
        [project],
    )?;
    source.execute(
        "INSERT INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch, outcome_kind,
          success_count, failure_count, recovery_count, correction_count, revert_count)
         VALUES (7, 0.93, 5, 'reviewed', 210, 500, 'recovery', 1, 2, 3, 4, 5)",
        [],
    )?;
    let export_dir = unique_temp_dir("markdown-export-metadata");
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
    target.execute_batch(
        "ALTER TABLE memories ADD COLUMN evidence_event_ids TEXT;
         ALTER TABLE memories ADD COLUMN source_candidate_id INTEGER;",
    )?;
    target.execute("INSERT INTO memory_candidates(id) VALUES (42)", [])?;
    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);

    let row: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<f64>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<i64>,
    ) = target.query_row(
        "SELECT source_project, target_project, owner_scope, owner_key, topic_domain,
                routing_confidence, routing_reason, context_class, expires_at_epoch,
                valid_from_epoch, valid_to_epoch, evidence_event_ids, source_candidate_id
         FROM memories WHERE topic_key = 'lesson-topic'",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
                row.get(11)?,
                row.get(12)?,
            ))
        },
    )?;
    assert_eq!(row.0.as_deref(), Some("/source"));
    assert_eq!(row.1.as_deref(), Some("/target"));
    assert_eq!(row.2.as_deref(), Some("workstream"));
    assert_eq!(row.3.as_deref(), Some("ws:1"));
    assert_eq!(row.4.as_deref(), Some("imports"));
    assert_eq!(row.5, Some(0.91));
    assert_eq!(row.6.as_deref(), Some("manual routing"));
    assert_eq!(row.7.as_deref(), Some("task_context"));
    assert_eq!(row.8, Some(400));
    assert_eq!(row.9, Some(125));
    assert_eq!(row.10, Some(350));
    assert_eq!(row.11.as_deref(), Some("[11,12]"));
    assert_eq!(row.12, Some(42));

    let lesson = crate::memory::lesson::get_lesson_metadata(&target, 1)?
        .expect("lesson metadata should round-trip");
    assert_eq!(lesson.confidence, 0.93);
    assert_eq!(lesson.reinforcement_count, 5);
    assert_eq!(lesson.source_evidence.as_deref(), Some("reviewed"));
    assert_eq!(lesson.last_reinforced_at_epoch, 210);
    assert_eq!(lesson.stale_after_epoch, Some(500));
    assert_eq!(lesson.outcome_kind, "recovery");
    assert_eq!(lesson.success_count, 1);
    assert_eq!(lesson.failure_count, 2);
    assert_eq!(lesson.recovery_count, 3);
    assert_eq!(lesson.correction_count, 4);
    assert_eq!(lesson.revert_count, 5);

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_export_refuses_non_empty_output_directory() -> Result<()> {
    let project = "/tmp/remem-markdown-non-empty";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    source.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES ('s1', ?1, 'non-empty-topic', 'Non empty export',
                 'Export should not overwrite existing markdown.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-non-empty");
    std::fs::create_dir_all(&export_dir)?;
    std::fs::write(export_dir.join("existing.md"), "manual edit")?;

    let error = export_markdown_archive(
        &source,
        MarkdownExportRequest {
            output: &export_dir,
            project,
            include_inactive: false,
            limit: 100,
        },
    )
    .expect_err("non-empty export directory must fail");
    assert!(
        error
            .to_string()
            .contains("export output directory must be empty"),
        "{error:#}"
    );

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_rejects_invalid_status_and_scope() {
    let bad_status = MarkdownMemoryDocument {
        metadata: sample_metadata("deleted", "project"),
        content: "Valid content.".to_string(),
    };
    let error = validate_markdown_metadata(&bad_status).expect_err("bad status must fail");
    assert!(error
        .to_string()
        .contains("unsupported markdown memory status"));

    let bad_scope = MarkdownMemoryDocument {
        metadata: sample_metadata("active", "repo"),
        content: "Valid content.".to_string(),
    };
    let error = validate_markdown_metadata(&bad_scope).expect_err("bad scope must fail");
    assert!(error
        .to_string()
        .contains("unsupported markdown memory scope"));
}

#[test]
fn markdown_import_rejects_relative_dates_without_reference_time() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let raw = format!(
        "{META_START}\n{}\n{META_END}\n\n# Yesterday decision\n\nYesterday we changed the importer.",
        serde_json::to_string_pretty(&MarkdownMemoryMetadata {
            reference_time_epoch: None,
            created_at_epoch: 0,
            updated_at_epoch: 0,
            title: "Yesterday decision".to_string(),
            topic_key: Some("relative".to_string()),
            ..sample_metadata("active", "project")
        })?
    );
    let doc = parse_markdown_memory(&raw)?;
    let error = validate_markdown_metadata(&doc).expect_err("relative date must fail");
    assert!(
        error
            .to_string()
            .contains("relative dates require a positive reference_time_epoch"),
        "{error:#}"
    );
    Ok(())
}
