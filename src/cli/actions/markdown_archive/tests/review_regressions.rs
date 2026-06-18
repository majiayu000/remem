use super::super::import_lookup;
use super::*;

fn create_memory_facts_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE memory_facts (
            id INTEGER PRIMARY KEY,
            project TEXT NOT NULL,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL CHECK (
                predicate IN (
                    'fixed_by', 'verified_by', 'supersedes', 'blocked_by',
                    'uses_file', 'uses_command', 'affects_project'
                )
            ),
            object TEXT NOT NULL,
            valid_from_epoch INTEGER,
            valid_to_epoch INTEGER,
            learned_at_epoch INTEGER NOT NULL,
            source_memory_id INTEGER REFERENCES memories(id) ON DELETE SET NULL,
            source_observation_id INTEGER,
            source_event_ids TEXT NOT NULL DEFAULT '[]',
            confidence REAL NOT NULL CHECK (confidence >= 0.0 AND confidence <= 1.0),
            supersedes_fact_id INTEGER REFERENCES memory_facts(id) ON DELETE SET NULL,
            status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'stale')),
            created_at_epoch INTEGER NOT NULL,
            updated_at_epoch INTEGER NOT NULL,
            invalidated_at_epoch INTEGER
        );",
    )?;
    Ok(())
}

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
fn markdown_import_source_id_survives_project_and_scope_edits() -> Result<()> {
    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    target.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (7, 'target-session', '/old-repo', 'scope-topic', 'Scoped preference',
                 'Stable source content.',
                 'preference', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [],
    )?;

    let export_dir = unique_temp_dir("markdown-export-scope-edit");
    std::fs::create_dir_all(&export_dir)?;
    let doc = MarkdownMemoryDocument {
        metadata: MarkdownMemoryMetadata {
            source_id: Some(7),
            project: "/new-repo".to_string(),
            topic_key: Some("scope-topic".to_string()),
            title: "Scoped preference".to_string(),
            memory_type: "preference".to_string(),
            created_at_epoch: 100,
            updated_at_epoch: 200,
            reference_time_epoch: Some(150),
            scope: "global".to_string(),
            source_content_hash: Some(import_lookup::markdown_source_content_hash(
                "Scoped preference",
                "Stable source content.",
                "preference",
                Some("scope-topic"),
            )),
            ..sample_metadata("active", "global")
        },
        content: "Stable source content.".to_string(),
    };
    std::fs::write(
        export_dir.join("scope-edit.md"),
        render_markdown_memory(&doc),
    )?;

    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 0);
    assert_eq!(stats.updated, 1);
    let row: (i64, String, String, String) = target.query_row(
        "SELECT COUNT(*), MAX(project), MAX(scope), MAX(owner_scope)
         FROM memories WHERE topic_key = 'scope-topic'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        row,
        (
            1,
            "/new-repo".to_string(),
            "global".to_string(),
            "user".to_string()
        )
    );

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_import_refreshes_source_hash_after_reindex_edits() -> Result<()> {
    let project = "/tmp/remem-markdown-source-refresh";
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (7, 'target-session', ?1, 'first-topic', 'First title',
                 'First source content.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-source-refresh");
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

    let mut first_edit = parse_markdown_memory(&std::fs::read_to_string(&path)?)?;
    first_edit.metadata.topic_key = Some("second-topic".to_string());
    first_edit.metadata.title = "Second title".to_string();
    first_edit.content = "Second source content.".to_string();
    std::fs::write(&path, render_markdown_memory(&first_edit))?;
    let first = import_markdown_archive(&conn, &export_dir, false)?;
    assert_eq!(first.imported, 0);
    assert_eq!(first.updated, 1);

    let mut second_edit = parse_markdown_memory(&std::fs::read_to_string(&path)?)?;
    second_edit.metadata.topic_key = Some("third-topic".to_string());
    second_edit.metadata.title = "Third title".to_string();
    second_edit.content = "Third source content.".to_string();
    std::fs::write(&path, render_markdown_memory(&second_edit))?;
    let second = import_markdown_archive(&conn, &export_dir, false)?;
    assert_eq!(second.imported, 0);
    assert_eq!(second.updated, 1);
    let row: (i64, String, String, String) = conn.query_row(
        "SELECT COUNT(*), MAX(topic_key), MAX(title), MAX(content)
         FROM memories WHERE id = 7 OR topic_key IN ('second-topic', 'third-topic')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(
        row,
        (
            1,
            "third-topic".to_string(),
            "Third title".to_string(),
            "Third source content.".to_string()
        )
    );

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_export_uses_context_visibility_and_current_filter() -> Result<()> {
    let project = "/tmp/remem-markdown-visible";
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_status,
          current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (10, 'repo', ?1, 'decision', 'hidden-topic', 'active', 999, 100, 100)",
        [project],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          target_project, owner_scope, owner_key)
         VALUES (1, 's1', '/source-repo', 'target-visible', 'Target visible',
                 'Repo-owned target_project content is visible.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project',
                 ?1, 'repo', '/source-repo')",
        [project],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          owner_scope, owner_key)
         VALUES (2, 's2', '/other-repo', 'global-visible', 'Global visible',
                 'User global content is visible.',
                 'preference', NULL, 'old search context',
                 110, 210, 160, 'active', 'main', 'global',
                 'user', 'user:default')",
        [],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          owner_scope, owner_key, state_key_id)
         VALUES (3, 's3', ?1, 'hidden-topic', 'Hidden non-current',
                 'Non-current active row should not export by default.',
                 'decision', NULL, 'old search context',
                 120, 220, 170, 'active', 'main', 'project',
                 'repo', ?1, 10)",
        [project],
    )?;

    let docs = load_export_memories(&conn, project, false, 100)?;
    let topics: Vec<_> = docs
        .iter()
        .filter_map(|doc| doc.metadata.topic_key.as_deref())
        .collect();
    assert_eq!(topics, vec!["target-visible", "global-visible"]);
    Ok(())
}

#[test]
fn markdown_global_import_matches_existing_global_topic_across_projects() -> Result<()> {
    let target = Connection::open_in_memory()?;
    setup_memory_schema(&target);
    target.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
          owner_scope, owner_key)
         VALUES ('target-session', '/original-repo', 'global-topic', 'Global preference',
                 'Original global content.',
                 'preference', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'global',
                 'user', 'user:default')",
        [],
    )?;
    let export_dir = unique_temp_dir("markdown-global-import");
    std::fs::create_dir_all(&export_dir)?;
    let doc = MarkdownMemoryDocument {
        metadata: MarkdownMemoryMetadata {
            source_id: None,
            project: "/exporting-repo".to_string(),
            topic_key: Some("global-topic".to_string()),
            title: "Global preference".to_string(),
            memory_type: "preference".to_string(),
            scope: "global".to_string(),
            source_project: Some("/exporting-repo".to_string()),
            target_project: None,
            owner_scope: Some("user".to_string()),
            owner_key: Some("user:default".to_string()),
            ..sample_metadata("active", "global")
        },
        content: "Edited global content.".to_string(),
    };
    std::fs::write(export_dir.join("global.md"), render_markdown_memory(&doc))?;

    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 0);
    assert_eq!(stats.updated, 1);
    let rows: (i64, String) = target.query_row(
        "SELECT COUNT(*), MAX(content) FROM memories
         WHERE topic_key = 'global-topic' AND COALESCE(scope, 'project') = 'global'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(rows, (1, "Edited global content.".to_string()));

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_round_trip_preserves_temporal_memory_facts() -> Result<()> {
    let project = "/tmp/remem-markdown-facts";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    create_memory_facts_schema(&source)?;
    source.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files, search_context,
          created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope)
         VALUES (7, 'source-session', ?1, 'fact-topic', 'Fact decision',
                 'Fact-backed markdown content.',
                 'decision', NULL, 'old search context',
                 100, 200, 150, 'active', 'main', 'project')",
        [project],
    )?;
    source.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_event_ids, confidence,
          supersedes_fact_id, status, created_at_epoch, updated_at_epoch, invalidated_at_epoch)
         VALUES (11, ?1, 'deploy', 'fixed_by', 'base-fix', 100, NULL,
                 120, 7, '[1,2]', 0.8, NULL, 'stale', 120, 140, 160)",
        [project],
    )?;
    source.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_event_ids, confidence,
          supersedes_fact_id, status, created_at_epoch, updated_at_epoch, invalidated_at_epoch)
         VALUES (12, ?1, 'deploy', 'verified_by', 'replacement-check', 170, NULL,
                 180, 7, '[3]', 0.9, 11, 'active', 180, 190, NULL)",
        [project],
    )?;
    let export_dir = unique_temp_dir("markdown-export-facts");
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
    create_memory_facts_schema(&target)?;
    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 1);
    let memory_id: i64 = target.query_row(
        "SELECT id FROM memories WHERE project = ?1 AND topic_key = 'fact-topic'",
        [project],
        |row| row.get(0),
    )?;
    let facts: Vec<(i64, String, String, Option<i64>, Option<i64>, String)> = {
        let mut stmt = target.prepare(
            "SELECT id, predicate, object, supersedes_fact_id, invalidated_at_epoch, source_event_ids
             FROM memory_facts
             WHERE source_memory_id = ?1
             ORDER BY object",
        )?;
        let rows = stmt.query_map([memory_id], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })?;
        crate::db::query::collect_rows(rows)?
    };
    assert_eq!(facts.len(), 2);
    let base = facts
        .iter()
        .find(|fact| fact.2 == "base-fix")
        .expect("base fact");
    let replacement = facts
        .iter()
        .find(|fact| fact.2 == "replacement-check")
        .expect("replacement fact");
    assert_eq!(base.1, "fixed_by");
    assert_eq!(base.4, Some(160));
    assert_eq!(base.5, "[]");
    assert_eq!(replacement.1, "verified_by");
    assert_eq!(replacement.3, Some(base.0));
    assert_ne!(replacement.3, Some(11));
    assert_eq!(replacement.5, "[]");

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

#[test]
fn markdown_round_trip_preserves_cross_memory_fact_supersession() -> Result<()> {
    let project = "/tmp/remem-markdown-cross-facts";
    let source = Connection::open_in_memory()?;
    setup_memory_schema(&source);
    create_memory_facts_schema(&source)?;
    for (id, topic, title, content) in [
        (
            7_i64,
            "base-fact-topic",
            "Base fact memory",
            "Base fact content.",
        ),
        (
            8_i64,
            "replacement-fact-topic",
            "Replacement fact memory",
            "Replacement fact content.",
        ),
    ] {
        source.execute(
            "INSERT INTO memories
             (id, session_id, project, topic_key, title, content, memory_type, files,
              search_context, created_at_epoch, updated_at_epoch, reference_time_epoch,
              status, branch, scope)
             VALUES (?1, 'source-session', ?2, ?3, ?4, ?5, 'decision', NULL,
                     'old search context', 100, 200, 150, 'active', 'main', 'project')",
            rusqlite::params![id, project, topic, title, content],
        )?;
    }
    source.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, learned_at_epoch, source_memory_id,
          source_event_ids, confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch)
         VALUES (11, ?1, 'deploy', 'fixed_by', 'base-fix', 120, 7,
                 '[1,2]', 0.8, NULL, 'stale', 120, 140)",
        [project],
    )?;
    source.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, learned_at_epoch, source_memory_id,
          source_event_ids, confidence, supersedes_fact_id, status, created_at_epoch, updated_at_epoch)
         VALUES (12, ?1, 'deploy', 'verified_by', 'replacement-check', 180, 8,
                 '[3]', 0.9, 11, 'active', 180, 190)",
        [project],
    )?;

    let export_dir = unique_temp_dir("markdown-export-cross-facts");
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
    create_memory_facts_schema(&target)?;
    let stats = import_markdown_archive(&target, &export_dir, false)?;
    assert_eq!(stats.imported, 2);
    let facts: Vec<(i64, String, Option<i64>, String)> = {
        let mut stmt = target.prepare(
            "SELECT id, object, supersedes_fact_id, source_event_ids
             FROM memory_facts
             ORDER BY object",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        crate::db::query::collect_rows(rows)?
    };
    let base = facts
        .iter()
        .find(|fact| fact.1 == "base-fix")
        .expect("base fact");
    let replacement = facts
        .iter()
        .find(|fact| fact.1 == "replacement-check")
        .expect("replacement fact");
    assert_eq!(replacement.2, Some(base.0));
    assert_ne!(replacement.2, Some(11));
    assert_eq!(base.3, "[]");
    assert_eq!(replacement.3, "[]");

    std::fs::remove_dir_all(&export_dir)
        .with_context(|| format!("remove {}", export_dir.display()))?;
    Ok(())
}

mod heading;
mod lifecycle;
