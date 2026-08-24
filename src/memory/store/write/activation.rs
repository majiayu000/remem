use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_memory_replacement_activated(
    conn: &Connection,
    _permit: &crate::memory::activation::ActiveMemoryWritePermit,
    superseded_id: Option<i64>,
    session_id: Option<&str>,
    project: &str,
    topic_key: Option<&str>,
    title: &str,
    content: &str,
    memory_type: &str,
    files: Option<&str>,
    branch: Option<&str>,
    scope: &str,
    source_trust: crate::memory::poisoning::SourceTrustClass,
    created_at_override: Option<i64>,
    reference_time_override: Option<i64>,
) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    let created_at = created_at_override.unwrap_or(now);
    let reference_time = reference_time_override
        .or(created_at_override)
        .unwrap_or(created_at);
    let (expires_at_epoch, valid_from_epoch) =
        crate::memory::lifecycle::ttl_metadata(memory_type, topic_key, content, now);
    let search_context = build_search_context(memory_type, topic_key, content, files);
    let fallback_source_hash = crate::memory::retrieval_enrichment::enrichment_source_hash(
        title,
        content,
        memory_type,
        topic_key,
        files,
    );
    let ownership = default_ownership(project, scope);
    let state_key = state_key::derive_state_key(memory_type, topic_key, title, content);

    with_memory_savepoint(conn, || {
        if let Some(memory_id) = superseded_id {
            let changed = conn.execute(
                "UPDATE memories
                 SET status = 'stale', valid_to_epoch = COALESCE(valid_to_epoch, ?1),
                     updated_at_epoch = ?1
                 WHERE id = ?2 AND status = 'active'",
                params![now, memory_id],
            )?;
            if changed != 1 {
                bail!("procedure replacement target is no longer active: {memory_id}");
            }
        }
        conn.execute(
            "INSERT INTO memories
             (session_id, project, topic_key, title, content, memory_type, files, search_context,
              search_context_fallback_source_hash,
              created_at_epoch, updated_at_epoch, reference_time_epoch, status, branch, scope,
              source_project, target_project, owner_scope, owner_key, context_class,
              source_trust_class, expires_at_epoch, valid_from_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'active', ?13, ?14,
                     ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                session_id,
                project,
                topic_key,
                title,
                content,
                memory_type,
                files,
                search_context,
                fallback_source_hash,
                created_at,
                now,
                reference_time,
                branch,
                scope,
                ownership.source_project,
                ownership.target_project,
                ownership.owner_scope,
                ownership.owner_key,
                ownership.context_class,
                source_trust.as_str(),
                expires_at_epoch,
                valid_from_epoch
            ],
        )?;
        let id = conn.last_insert_rowid();
        attach_state_key(conn, id, memory_type, &ownership, state_key.as_ref(), now)?;
        refresh_memory_entities(conn, id, title, content)?;
        refresh_memory_embedding(conn, id, title, content, memory_type, topic_key)?;
        Ok(id)
    })
}
