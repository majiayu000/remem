use super::{column_exists, MarkdownLessonMetadata, MarkdownMemoryDocument};
use anyhow::{bail, Context, Result};
use rusqlite::Connection;

pub(super) struct MarkdownOwnership<'a> {
    pub(super) source_project: &'a str,
    pub(super) target_project: Option<&'a str>,
    pub(super) owner_scope: &'a str,
    pub(super) owner_key: &'a str,
    pub(super) context_class: &'a str,
}

pub(super) fn markdown_ownership(doc: &MarkdownMemoryDocument) -> MarkdownOwnership<'_> {
    let fallback =
        crate::memory::store::default_ownership(&doc.metadata.project, &doc.metadata.scope);
    MarkdownOwnership {
        source_project: doc
            .metadata
            .source_project
            .as_deref()
            .unwrap_or(fallback.source_project),
        target_project: doc
            .metadata
            .target_project
            .as_deref()
            .or(fallback.target_project),
        owner_scope: doc
            .metadata
            .owner_scope
            .as_deref()
            .unwrap_or(fallback.owner_scope),
        owner_key: doc
            .metadata
            .owner_key
            .as_deref()
            .unwrap_or(fallback.owner_key),
        context_class: doc
            .metadata
            .context_class
            .as_deref()
            .unwrap_or(fallback.context_class),
    }
}

pub(super) fn update_markdown_memory(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
) -> Result<()> {
    if doc.metadata.status != "active" {
        return update_markdown_memory_row(conn, memory_id, doc, topic_key);
    }
    validate_active_markdown_route(conn, memory_id, doc)?;
    let request = markdown_activation_request(
        doc,
        topic_key,
        "markdown_update",
        &format!("memory:{memory_id}"),
    )?;
    crate::memory::activation::execute_one(conn, &request, |_permit| {
        update_markdown_memory_row(conn, memory_id, doc, topic_key)?;
        Ok(memory_id)
    })?;
    Ok(())
}

pub(super) fn insert_markdown_memory(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
) -> Result<i64> {
    if doc.metadata.status != "active" {
        return insert_markdown_memory_row(conn, doc, topic_key);
    }
    let request = markdown_activation_request(doc, topic_key, "markdown_insert", "new")?;
    Ok(
        crate::memory::activation::execute_one(conn, &request, |_permit| {
            insert_markdown_memory_row(conn, doc, topic_key)
        })?
        .memory_id,
    )
}

fn markdown_activation_request(
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
    operation: &str,
    identity: &str,
) -> Result<crate::memory::activation::ActiveMemoryWriteRequest> {
    if let Some(matched) = crate::memory::poisoning::scan_instruction_pattern(&format!(
        "{}\n{}",
        doc.metadata.title, doc.content
    )) {
        bail!(
            "markdown import payload matched instruction-pattern {}@{}",
            matched.pattern_id,
            matched.pattern_set_version
        );
    }
    let ownership = markdown_ownership(doc);
    let serialized = serde_json::to_string(&(&doc.metadata, &doc.content, topic_key))?;
    let payload_sha256 = crate::memory::activation::payload_sha256(&[&serialized]);
    Ok(crate::memory::activation::ActiveMemoryWriteRequest {
        activation_id: crate::memory::activation::ephemeral_activation_id(
            operation,
            &payload_sha256,
        ),
        route_kind: crate::memory::activation::ActivationRouteKind::SupplementalSave,
        actor_kind: crate::memory::activation::ActivationActorKind::Operator,
        source_operation: operation.to_string(),
        source_trust: crate::memory::poisoning::SourceTrustClass::RepoFile,
        source_project: ownership.source_project.to_string(),
        route: crate::memory::activation::ActiveMemoryRoute {
            project: doc.metadata.project.clone(),
            branch: doc.metadata.branch.clone(),
            scope: doc.metadata.scope.clone(),
            owner_scope: ownership.owner_scope.to_string(),
            owner_key: ownership.owner_key.to_string(),
            target_project: ownership.target_project.map(str::to_string),
        },
        provenance_kind: crate::memory::activation::ActivationProvenanceKind::SupplementalSave,
        provenance_ref: format!(
            "operator:markdown:{identity}:source-hash:{}",
            doc.metadata
                .source_content_hash
                .as_deref()
                .unwrap_or("unbound")
        ),
        payload_sha256,
        expected_memory: crate::memory::activation::ExpectedActiveMemory::new(
            &doc.metadata.title,
            &doc.content,
            &doc.metadata.memory_type,
        )
        .with_topic_key(topic_key)
        .with_files(doc.metadata.files.as_deref()),
        poisoning_verdict: crate::memory::activation::ActivationPoisoningVerdict::Clean,
        superseded_ids: Vec::new(),
    })
}

fn validate_active_markdown_route(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
) -> Result<()> {
    let ownership = markdown_ownership(doc);
    let route_matches: bool = conn.query_row(
        "SELECT project = ?2
                AND branch IS ?3 AND COALESCE(scope, 'project') = ?4
                AND COALESCE(owner_scope,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user' ELSE 'repo' END) = ?5
                AND COALESCE(owner_key,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN 'user:default' ELSE project END) = ?6
                AND COALESCE(target_project,
                    CASE WHEN COALESCE(scope, 'project') = 'global' THEN NULL ELSE project END) IS ?7
         FROM memories WHERE id = ?1",
        rusqlite::params![
            memory_id,
            doc.metadata.project,
            doc.metadata.branch,
            doc.metadata.scope,
            ownership.owner_scope,
            ownership.owner_key,
            ownership.target_project,
        ],
        |row| row.get(0),
    )?;
    if !route_matches {
        bail!("markdown import cannot activate a memory across project/branch/owner route");
    }
    Ok(())
}

fn update_markdown_memory_row(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
) -> Result<()> {
    conn.execute_batch("SAVEPOINT remem_update_markdown_memory")?;
    let result = (|| -> Result<()> {
        let reference_time_epoch = doc
            .metadata
            .reference_time_epoch
            .unwrap_or(doc.metadata.created_at_epoch);
        let ownership = markdown_ownership(doc);
        let updated_at_epoch = super::change_detection::markdown_update_epoch(
            conn,
            memory_id,
            doc,
            topic_key,
            reference_time_epoch,
            &ownership,
        )?;
        let search_context = crate::memory::search_context::build_search_context(
            &doc.metadata.memory_type,
            topic_key,
            &doc.content,
            doc.metadata.files.as_deref(),
        );
        conn.execute(
            "UPDATE memories SET project = ?1, topic_key = ?2, title = ?3, content = ?4,
                 memory_type = ?5, files = ?6, search_context = ?7, created_at_epoch = ?8,
                 updated_at_epoch = ?9, reference_time_epoch = ?10, status = ?11,
                 branch = ?12, scope = ?13, source_project = ?14, target_project = ?15,
                 owner_scope = ?16, owner_key = ?17, topic_domain = ?18,
                 routing_confidence = ?19, routing_reason = ?20, context_class = ?21,
                 expires_at_epoch = ?22, valid_from_epoch = ?23, valid_to_epoch = ?24
             WHERE id = ?25",
            rusqlite::params![
                doc.metadata.project,
                topic_key,
                doc.metadata.title,
                doc.content,
                doc.metadata.memory_type,
                doc.metadata.files,
                search_context,
                doc.metadata.created_at_epoch,
                updated_at_epoch,
                reference_time_epoch,
                doc.metadata.status,
                doc.metadata.branch,
                doc.metadata.scope,
                ownership.source_project,
                ownership.target_project,
                ownership.owner_scope,
                ownership.owner_key,
                doc.metadata.topic_domain,
                doc.metadata.routing_confidence,
                doc.metadata.routing_reason,
                ownership.context_class,
                doc.metadata.expires_at_epoch,
                doc.metadata.valid_from_epoch,
                doc.metadata.valid_to_epoch,
                memory_id,
            ],
        )?;
        update_optional_memory_provenance(conn, memory_id, doc)?;
        super::refresh_markdown_memory_indexes(
            conn,
            memory_id,
            doc,
            topic_key,
            &ownership,
            updated_at_epoch,
        )?;
        Ok(())
    })();
    finish_savepoint(conn, "remem_update_markdown_memory", result)
}

fn insert_markdown_memory_row(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
    topic_key: Option<&str>,
) -> Result<i64> {
    conn.execute_batch("SAVEPOINT remem_import_markdown_memory")?;
    let result = (|| -> Result<i64> {
        let reference_time_epoch = doc
            .metadata
            .reference_time_epoch
            .unwrap_or(doc.metadata.created_at_epoch);
        let search_context = crate::memory::search_context::build_search_context(
            &doc.metadata.memory_type,
            topic_key,
            &doc.content,
            doc.metadata.files.as_deref(),
        );
        let ownership = markdown_ownership(doc);
        conn.execute(
            "INSERT INTO memories
             (session_id, project, topic_key, title, content, memory_type, files, search_context,
              created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
              source_project, target_project, owner_scope, owner_key, topic_domain,
              routing_confidence, routing_reason, context_class, expires_at_epoch,
              valid_from_epoch, valid_to_epoch)
             VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            rusqlite::params![
                doc.metadata.project,
                topic_key,
                doc.metadata.title,
                doc.content,
                doc.metadata.memory_type,
                doc.metadata.files,
                search_context,
                doc.metadata.created_at_epoch,
                doc.metadata.updated_at_epoch,
                reference_time_epoch,
                doc.metadata.status,
                doc.metadata.branch,
                doc.metadata.scope,
                ownership.source_project,
                ownership.target_project,
                ownership.owner_scope,
                ownership.owner_key,
                doc.metadata.topic_domain,
                doc.metadata.routing_confidence,
                doc.metadata.routing_reason,
                ownership.context_class,
                doc.metadata.expires_at_epoch,
                doc.metadata.valid_from_epoch,
                doc.metadata.valid_to_epoch,
            ],
        )?;
        let memory_id = conn.last_insert_rowid();
        update_optional_memory_provenance(conn, memory_id, doc)?;
        super::refresh_markdown_memory_indexes(
            conn,
            memory_id,
            doc,
            topic_key,
            &ownership,
            doc.metadata.updated_at_epoch,
        )?;
        Ok(memory_id)
    })();
    finish_savepoint(conn, "remem_import_markdown_memory", result)
}

fn finish_savepoint<T>(conn: &Connection, name: &str, result: Result<T>) -> Result<T> {
    match result {
        Ok(value) => {
            conn.execute_batch(&format!("RELEASE SAVEPOINT {name}"))?;
            Ok(value)
        }
        Err(error) => {
            conn.execute_batch(&format!(
                "ROLLBACK TO SAVEPOINT {name}; RELEASE SAVEPOINT {name}"
            ))
            .context(format!(
                "rollback markdown memory mutation after failure: {error}"
            ))?;
            Err(error)
        }
    }
}

pub(super) fn update_optional_memory_provenance(
    conn: &Connection,
    memory_id: i64,
    _doc: &MarkdownMemoryDocument,
) -> Result<()> {
    if column_exists(conn, "memories", "evidence_event_ids")? {
        conn.execute(
            "UPDATE memories SET evidence_event_ids = ?1 WHERE id = ?2",
            rusqlite::params![Option::<String>::None, memory_id],
        )?;
    }
    if column_exists(conn, "memories", "source_candidate_id")? {
        conn.execute(
            "UPDATE memories SET source_candidate_id = ?1 WHERE id = ?2",
            rusqlite::params![Option::<i64>::None, memory_id],
        )?;
    }
    if column_exists(conn, "memories", "source_trust_class")? {
        conn.execute(
            "UPDATE memories SET source_trust_class = 'repo_file' WHERE id = ?1",
            [memory_id],
        )?;
    }
    Ok(())
}

pub(super) fn upsert_markdown_lesson_metadata(
    conn: &Connection,
    memory_id: i64,
    doc: &MarkdownMemoryDocument,
    updated_at_epoch: i64,
) -> Result<()> {
    let fallback = default_markdown_lesson_metadata(updated_at_epoch);
    let lesson = doc.metadata.lesson.as_ref().unwrap_or(&fallback);
    conn.execute(
        "INSERT INTO memory_lessons
         (memory_id, confidence, reinforcement_count, source_evidence,
          last_reinforced_at_epoch, stale_after_epoch, outcome_kind,
          success_count, failure_count, recovery_count, correction_count, revert_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(memory_id) DO UPDATE SET
           confidence = excluded.confidence,
           reinforcement_count = excluded.reinforcement_count,
           source_evidence = excluded.source_evidence,
           last_reinforced_at_epoch = excluded.last_reinforced_at_epoch,
           stale_after_epoch = excluded.stale_after_epoch,
           outcome_kind = excluded.outcome_kind,
           success_count = excluded.success_count,
           failure_count = excluded.failure_count,
           recovery_count = excluded.recovery_count,
           correction_count = excluded.correction_count,
           revert_count = excluded.revert_count",
        rusqlite::params![
            memory_id,
            lesson.confidence,
            lesson.reinforcement_count,
            lesson.source_evidence,
            lesson.last_reinforced_at_epoch,
            lesson.stale_after_epoch,
            lesson.outcome_kind,
            lesson.success_count,
            lesson.failure_count,
            lesson.recovery_count,
            lesson.correction_count,
            lesson.revert_count,
        ],
    )?;
    Ok(())
}

pub(super) fn default_markdown_lesson_metadata(updated_at_epoch: i64) -> MarkdownLessonMetadata {
    MarkdownLessonMetadata {
        confidence: 0.7,
        reinforcement_count: 1,
        source_evidence: Some("markdown_import".to_string()),
        last_reinforced_at_epoch: updated_at_epoch,
        stale_after_epoch: None,
        outcome_kind: "unknown".to_string(),
        success_count: 0,
        failure_count: 0,
        recovery_count: 0,
        correction_count: 0,
        revert_count: 0,
    }
}
