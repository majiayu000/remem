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
