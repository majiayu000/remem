use super::persist::markdown_ownership;
use super::MarkdownMemoryDocument;
use anyhow::Result;
use rusqlite::Connection;

pub(super) fn runtime_memory_id_by_source(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
) -> Result<Option<i64>> {
    let Some(source_id) = doc.metadata.source_id else {
        return Ok(None);
    };
    let Some(source_content_hash) = doc.metadata.source_content_hash.as_deref() else {
        return Ok(None);
    };
    let result = conn.query_row(
        "SELECT id, title, content, memory_type, topic_key FROM memories
         WHERE id = ?1
           AND project = ?2
           AND COALESCE(scope, 'project') = ?3
           AND created_at_epoch = ?4
           AND COALESCE(reference_time_epoch, created_at_epoch) = ?5
         LIMIT 1",
        rusqlite::params![
            source_id,
            doc.metadata.project,
            doc.metadata.scope,
            doc.metadata.created_at_epoch,
            doc.metadata
                .reference_time_epoch
                .unwrap_or(doc.metadata.created_at_epoch),
        ],
        |row| {
            Ok(ExistingSourceCandidate {
                id: row.get(0)?,
                title: row.get(1)?,
                content: row.get(2)?,
                memory_type: row.get(3)?,
                topic_key: row.get(4)?,
            })
        },
    );
    match result {
        Ok(existing) if existing.matches_source(doc, source_content_hash) => Ok(Some(existing.id)),
        Ok(_) => Ok(None),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

struct ExistingSourceCandidate {
    id: i64,
    title: String,
    content: String,
    memory_type: String,
    topic_key: Option<String>,
}

impl ExistingSourceCandidate {
    fn matches_source(&self, doc: &MarkdownMemoryDocument, source_content_hash: &str) -> bool {
        markdown_source_content_hash(
            &self.title,
            &self.content,
            &self.memory_type,
            self.topic_key.as_deref(),
        ) == source_content_hash
            || (self.title == doc.metadata.title
                && self.content == doc.content
                && self.memory_type == doc.metadata.memory_type)
    }
}

pub(super) fn markdown_source_content_hash(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> String {
    crate::retrieval::embedding::embedding_content_hash(title, content, memory_type, topic_key)
}

pub(super) fn runtime_memory_id(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
    topic_key: &str,
) -> Result<Option<i64>> {
    if doc.metadata.scope == "global" {
        return runtime_global_memory_id(conn, doc, topic_key);
    }
    let result = conn.query_row(
        "SELECT id FROM memories
         WHERE project = ?1
           AND topic_key = ?2
           AND COALESCE(scope, 'project') = ?3
         ORDER BY CASE status
             WHEN 'active' THEN 0
             WHEN 'stale' THEN 1
             WHEN 'archived' THEN 2
             ELSE 3
           END,
           updated_at_epoch DESC,
           id DESC
         LIMIT 1",
        rusqlite::params![doc.metadata.project, topic_key, doc.metadata.scope],
        |row| row.get::<_, i64>(0),
    );
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn runtime_global_memory_id(
    conn: &Connection,
    doc: &MarkdownMemoryDocument,
    topic_key: &str,
) -> Result<Option<i64>> {
    let ownership = markdown_ownership(doc);
    let result = conn.query_row(
        "SELECT id FROM memories
         WHERE topic_key = ?1
           AND COALESCE(scope, 'project') = 'global'
           AND COALESCE(owner_scope, ?2) = ?2
           AND COALESCE(owner_key, ?3) = ?3
         ORDER BY CASE status
             WHEN 'active' THEN 0
             WHEN 'stale' THEN 1
             WHEN 'archived' THEN 2
             ELSE 3
           END,
           updated_at_epoch DESC,
           id DESC
         LIMIT 1",
        rusqlite::params![topic_key, ownership.owner_scope, ownership.owner_key],
        |row| row.get::<_, i64>(0),
    );
    match result {
        Ok(id) => Ok(Some(id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
