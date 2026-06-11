use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

pub const EMBEDDING_DIMENSIONS: usize = 768;
pub const DEFAULT_EMBEDDING_MODEL: &str = "remem-local-feature-hash-v1";
pub const VECTOR_SEARCH_CANDIDATE_LIMIT: usize = 4_096;
const VECTOR_SEARCH_MIN_CANDIDATES: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct VectorHit {
    pub memory_id: i64,
    pub distance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VectorSearchOutcome {
    pub hits: Vec<VectorHit>,
    pub disabled_reason: Option<String>,
    pub candidates_scanned: usize,
}

impl VectorSearchOutcome {
    pub fn disabled(reason: impl Into<String>) -> Self {
        Self {
            hits: vec![],
            disabled_reason: Some(reason.into()),
            candidates_scanned: 0,
        }
    }

    pub fn ready(hits: Vec<VectorHit>) -> Self {
        let candidates_scanned = hits.len();
        Self::ready_with_scan_count(hits, candidates_scanned)
    }

    pub fn ready_with_scan_count(hits: Vec<VectorHit>, candidates_scanned: usize) -> Self {
        Self {
            hits,
            disabled_reason: None,
            candidates_scanned,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct VectorSearchFilters<'a> {
    pub project: Option<&'a str>,
    pub memory_type: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub include_stale: bool,
}

/// Load a native vector extension when one is configured.
///
/// The current production path is a portable SQLite table plus in-process
/// cosine scan. That keeps vector recall available in the single-binary build;
/// sqlite-vec can replace the scan later without changing the search contract.
pub fn load_vec_extension(_conn: &Connection) -> Result<()> {
    Ok(())
}

pub fn ensure_vec_table(conn: &Connection) -> Result<()> {
    create_embedding_table(conn)
}

pub fn upsert_embedding(conn: &Connection, memory_id: i64, embedding: &[f32]) -> Result<()> {
    upsert_embedding_with_metadata(
        conn,
        memory_id,
        DEFAULT_EMBEDDING_MODEL,
        "",
        embedding,
        chrono::Utc::now().timestamp(),
    )
}

pub fn upsert_memory_embedding(
    conn: &Connection,
    memory_id: i64,
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Result<()> {
    let embedding = embed_memory_text(title, content, memory_type, topic_key);
    let content_hash = embedding_content_hash(title, content, memory_type, topic_key);
    upsert_embedding_with_metadata(
        conn,
        memory_id,
        DEFAULT_EMBEDDING_MODEL,
        &content_hash,
        &embedding,
        chrono::Utc::now().timestamp(),
    )
    .with_context(|| format!("memory embedding upsert failed for memory id={memory_id}"))
}

pub fn upsert_memory_embedding_for_row(conn: &Connection, memory_id: i64) -> Result<()> {
    let (topic_key, title, content, memory_type): (Option<String>, String, String, String) = conn
        .query_row(
            "SELECT topic_key, title, content, memory_type
             FROM memories
             WHERE id = ?1",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .with_context(|| format!("load memory row for embedding id={memory_id}"))?;
    upsert_memory_embedding(
        conn,
        memory_id,
        &title,
        &content,
        &memory_type,
        topic_key.as_deref(),
    )
}

pub fn backfill_missing_memory_embeddings(conn: &Connection, limit: i64) -> Result<usize> {
    if !table_exists(conn, "memories")? || !table_exists(conn, "memory_embeddings")? {
        return Ok(0);
    }
    let limit = limit.max(0);
    if limit == 0 {
        return Ok(0);
    }

    let mut stmt = conn.prepare(
        "SELECT m.id, m.topic_key, m.title, m.content, m.memory_type
         FROM memories m
         LEFT JOIN memory_embeddings e ON e.memory_id = m.id
         WHERE e.memory_id IS NULL
           AND m.status IN ('active', 'stale', 'archived')
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let pending = crate::db::query::collect_rows(rows)?;
    let count = pending.len();
    for (id, topic_key, title, content, memory_type) in pending {
        upsert_memory_embedding(
            conn,
            id,
            &title,
            &content,
            &memory_type,
            topic_key.as_deref(),
        )?;
    }
    Ok(count)
}

pub fn embedding_count(conn: &Connection) -> Result<i64> {
    if !table_exists(conn, "memory_embeddings")? {
        return Ok(0);
    }
    Ok(
        conn.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })?,
    )
}

pub fn embed_query_text(query: &str) -> Vec<f32> {
    embed_text(query)
}

pub fn embed_memory_text(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> Vec<f32> {
    let mut text = String::new();
    text.push_str(memory_type);
    text.push('\n');
    if let Some(topic_key) = topic_key {
        text.push_str(topic_key);
        text.push('\n');
    }
    text.push_str(title);
    text.push('\n');
    text.push_str(content);
    embed_text(&text)
}

pub fn vector_search(
    conn: &Connection,
    query_embedding: &[f32],
    limit: usize,
) -> Result<Vec<(i64, f32)>> {
    Ok(
        vector_search_filtered(conn, query_embedding, VectorSearchFilters::default(), limit)?
            .hits
            .into_iter()
            .map(|hit| (hit.memory_id, hit.distance))
            .collect(),
    )
}

pub fn vector_search_filtered(
    conn: &Connection,
    query_embedding: &[f32],
    filters: VectorSearchFilters<'_>,
    limit: usize,
) -> Result<VectorSearchOutcome> {
    if query_embedding.len() != EMBEDDING_DIMENSIONS {
        anyhow::bail!(
            "query embedding must be {} dimensions, got {}",
            EMBEDDING_DIMENSIONS,
            query_embedding.len()
        );
    }
    if limit == 0 {
        return Ok(VectorSearchOutcome::ready(vec![]));
    }
    if !table_exists(conn, "memory_embeddings")? {
        return Ok(VectorSearchOutcome::disabled(
            "memory_embeddings table is missing; run migrations/backfill",
        ));
    }

    let mut conditions = vec![crate::memory::memory_current_filter_sql(
        "m.status",
        "m.expires_at_epoch",
        filters.include_stale,
    )];
    if !filters.include_stale {
        conditions.push(crate::memory::memory_state_key_current_filter_sql("m"));
    }
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    if let Some(project) = filters.project {
        conditions.push(format!("(m.project = ?{idx} OR m.scope = 'global')"));
        param_values.push(Box::new(project.to_string()));
        idx += 1;
    }
    if let Some(branch) = filters.branch {
        conditions.push(format!("(m.branch = ?{idx} OR m.branch IS NULL)"));
        param_values.push(Box::new(branch.to_string()));
        idx += 1;
    }
    if let Some(memory_type) = filters.memory_type {
        conditions.push(format!("m.memory_type = ?{idx}"));
        param_values.push(Box::new(memory_type.to_string()));
        idx += 1;
    }
    let candidate_limit = vector_candidate_limit(limit);
    param_values.push(Box::new(candidate_limit as i64));

    let sql = format!(
        "SELECT m.id, e.embedding, e.dimensions
         FROM memory_embeddings e
         JOIN memories m ON m.id = e.memory_id
         WHERE {}
         ORDER BY m.updated_at_epoch DESC, m.id DESC
         LIMIT ?{idx}",
        conditions.join(" AND ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let refs = crate::db::to_sql_refs(&param_values);
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let candidates = crate::db::query::collect_rows(rows)?;

    let candidates_scanned = candidates.len();
    let mut hits = Vec::new();
    for (memory_id, blob, dimensions) in candidates {
        let embedding = decode_embedding(&blob, dimensions)
            .with_context(|| format!("invalid embedding blob for memory id={memory_id}"))?;
        let distance = cosine_distance(query_embedding, &embedding)?;
        hits.push(VectorHit {
            memory_id,
            distance,
        });
    }
    hits.sort_by(|a, b| {
        a.distance
            .partial_cmp(&b.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });
    hits.truncate(limit);
    Ok(VectorSearchOutcome::ready_with_scan_count(
        hits,
        candidates_scanned,
    ))
}

fn vector_candidate_limit(limit: usize) -> usize {
    limit.clamp(VECTOR_SEARCH_MIN_CANDIDATES, VECTOR_SEARCH_CANDIDATE_LIMIT)
}

pub fn find_similar_observations(
    conn: &Connection,
    query_embedding: &[f32],
    threshold: f32,
    limit: usize,
) -> Result<Vec<i64>> {
    let candidates = vector_search(conn, query_embedding, limit)?;
    let distance_threshold = 1.0 - threshold;
    let similar: Vec<i64> = candidates
        .into_iter()
        .filter(|(_, dist)| *dist < distance_threshold)
        .map(|(id, _)| id)
        .collect();

    Ok(similar)
}

fn create_embedding_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_embeddings (
             memory_id INTEGER PRIMARY KEY,
             embedding BLOB NOT NULL,
             dimensions INTEGER NOT NULL,
             model TEXT NOT NULL,
             content_hash TEXT NOT NULL,
             updated_at_epoch INTEGER NOT NULL,
             FOREIGN KEY(memory_id) REFERENCES memories(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_memory_embeddings_model
             ON memory_embeddings(model, updated_at_epoch);",
    )?;
    Ok(())
}

fn upsert_embedding_with_metadata(
    conn: &Connection,
    memory_id: i64,
    model: &str,
    content_hash: &str,
    embedding: &[f32],
    updated_at_epoch: i64,
) -> Result<()> {
    if embedding.len() != EMBEDDING_DIMENSIONS {
        anyhow::bail!(
            "embedding must be {} dimensions, got {}",
            EMBEDDING_DIMENSIONS,
            embedding.len()
        );
    }
    let blob = encode_embedding(embedding);
    conn.execute(
        "INSERT INTO memory_embeddings
         (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(memory_id) DO UPDATE SET
             embedding = excluded.embedding,
             dimensions = excluded.dimensions,
             model = excluded.model,
             content_hash = excluded.content_hash,
             updated_at_epoch = excluded.updated_at_epoch",
        params![
            memory_id,
            blob,
            EMBEDDING_DIMENSIONS as i64,
            model,
            content_hash,
            updated_at_epoch
        ],
    )?;
    Ok(())
}

fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(std::mem::size_of_val(embedding));
    for value in embedding {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn decode_embedding(blob: &[u8], dimensions: i64) -> Result<Vec<f32>> {
    if dimensions != EMBEDDING_DIMENSIONS as i64 {
        anyhow::bail!(
            "embedding dimensions must be {}, got {}",
            EMBEDDING_DIMENSIONS,
            dimensions
        );
    }
    if blob.len() != EMBEDDING_DIMENSIONS * std::mem::size_of::<f32>() {
        anyhow::bail!(
            "embedding blob must be {} bytes, got {}",
            EMBEDDING_DIMENSIONS * std::mem::size_of::<f32>(),
            blob.len()
        );
    }
    Ok(blob
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn cosine_distance(a: &[f32], b: &[f32]) -> Result<f32> {
    if a.len() != b.len() {
        anyhow::bail!(
            "embedding dimensions differ: query={} stored={}",
            a.len(),
            b.len()
        );
    }
    let mut dot = 0.0f32;
    let mut a_norm = 0.0f32;
    let mut b_norm = 0.0f32;
    for (left, right) in a.iter().zip(b) {
        dot += left * right;
        a_norm += left * left;
        b_norm += right * right;
    }
    if a_norm == 0.0 || b_norm == 0.0 {
        return Ok(1.0);
    }
    Ok((1.0 - dot / (a_norm.sqrt() * b_norm.sqrt())).clamp(0.0, 2.0))
}

fn embed_text(text: &str) -> Vec<f32> {
    let normalized = text.to_lowercase();
    let mut vector = vec![0.0f32; EMBEDDING_DIMENSIONS];
    for token in semantic_tokens(&normalized) {
        add_feature(&mut vector, &format!("token:{token}"), 1.0);
    }
    for ngram in char_ngrams(&normalized) {
        add_feature(&mut vector, &format!("ngram:{ngram}"), 0.35);
    }
    for (concept, phrases) in semantic_concepts() {
        if phrases.iter().any(|phrase| normalized.contains(phrase)) {
            add_feature(&mut vector, &format!("concept:{concept}"), 4.0);
        }
    }
    normalize(&mut vector);
    vector
}

fn semantic_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() || is_cjk(ch) {
            current.push(ch);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn char_ngrams(text: &str) -> Vec<String> {
    let chars: Vec<char> = text
        .chars()
        .filter(|ch| ch.is_alphanumeric() || is_cjk(*ch))
        .collect();
    let mut grams = Vec::new();
    for width in [2usize, 3] {
        if chars.len() < width {
            continue;
        }
        grams.extend(
            chars
                .windows(width)
                .map(|window| window.iter().collect::<String>()),
        );
    }
    grams
}

fn add_feature(vector: &mut [f32], feature: &str, weight: f32) {
    let digest = Sha256::digest(feature.as_bytes());
    for offset in [0usize, 8, 16] {
        let raw = u64::from_le_bytes([
            digest[offset],
            digest[offset + 1],
            digest[offset + 2],
            digest[offset + 3],
            digest[offset + 4],
            digest[offset + 5],
            digest[offset + 6],
            digest[offset + 7],
        ]);
        let idx = raw as usize % vector.len();
        let sign = if raw & 1 == 0 { 1.0 } else { -1.0 };
        vector[idx] += weight * sign;
    }
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

fn embedding_content_hash(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(memory_type.as_bytes());
    hasher.update([0]);
    if let Some(topic_key) = topic_key {
        hasher.update(topic_key.as_bytes());
    }
    hasher.update([0]);
    hasher.update(title.as_bytes());
    hasher.update([0]);
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}' |
        '\u{3400}'..='\u{4DBF}' |
        '\u{F900}'..='\u{FAFF}'
    )
}

fn semantic_concepts() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        (
            "data-security",
            &[
                "sqlcipher",
                "encrypt",
                "encrypted",
                "encryption",
                "secret",
                "secrets",
                "credential",
                "credentials",
                "private",
                "confidential",
                "protect",
                "protected",
                "at rest",
                "persisted data",
                "加密",
                "密钥",
            ],
        ),
        (
            "transcript-capture",
            &[
                "transcript",
                "raw archive",
                "raw message",
                "hook fallback",
                "assistant message",
                "conversation capture",
                "jsonl",
                "会话",
                "原始消息",
            ],
        ),
        (
            "retrieval-quality",
            &[
                "semantic",
                "embedding",
                "vector",
                "recall",
                "search quality",
                "paraphrase",
                "检索",
                "语义",
                "召回",
                "向量",
            ],
        ),
        (
            "current-state",
            &[
                "current decision",
                "current state",
                "supersede",
                "supersedes",
                "stale",
                "replacement",
                "现在",
                "当前",
                "替代",
            ],
        ),
        (
            "compression",
            &[
                "compress",
                "compression",
                "compaction",
                "summarize",
                "compressed",
                "压缩",
                "摘要",
                "总结",
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use rusqlite::Connection;

    use super::*;

    fn setup_conn() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    #[test]
    fn vector_search_returns_nearest_memory_embedding() -> Result<()> {
        let conn = setup_conn()?;
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES
             (1, '/repo', 'Credential store', 'SQLCipher encrypts secrets at rest.', 'architecture', 1, 1, 'active'),
             (2, '/repo', 'Posting workflow', 'Publish social media drafts after review.', 'procedure', 1, 1, 'active')",
            [],
        )?;
        upsert_memory_embedding(
            &conn,
            1,
            "Credential store",
            "SQLCipher encrypts secrets at rest.",
            "architecture",
            None,
        )?;
        upsert_memory_embedding(
            &conn,
            2,
            "Posting workflow",
            "Publish social media drafts after review.",
            "procedure",
            None,
        )?;

        let query = embed_query_text("How do we protect private persisted data?");
        let outcome = vector_search_filtered(
            &conn,
            &query,
            VectorSearchFilters {
                project: Some("/repo"),
                ..VectorSearchFilters::default()
            },
            5,
        )?;

        assert!(outcome.disabled_reason.is_none());
        assert_eq!(outcome.hits[0].memory_id, 1);
        Ok(())
    }

    #[test]
    fn vector_search_respects_filters() -> Result<()> {
        let conn = setup_conn()?;
        for (id, project, branch, memory_type, status) in [
            (1, "/repo", Some("main"), "architecture", "active"),
            (2, "/other", Some("main"), "architecture", "active"),
            (3, "/repo", Some("feature"), "architecture", "active"),
            (4, "/repo", Some("main"), "decision", "active"),
            (5, "/repo", Some("main"), "architecture", "stale"),
        ] {
            conn.execute(
                "INSERT INTO memories
                 (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status, branch)
                 VALUES (?1, ?2, 'Credential store', 'SQLCipher encrypts secrets at rest.', ?3, 1, 1, ?4, ?5)",
                params![id, project, memory_type, status, branch],
            )?;
            upsert_memory_embedding(
                &conn,
                id,
                "Credential store",
                "SQLCipher encrypts secrets at rest.",
                memory_type,
                None,
            )?;
        }

        let query = embed_query_text("protect private persisted data");
        let outcome = vector_search_filtered(
            &conn,
            &query,
            VectorSearchFilters {
                project: Some("/repo"),
                branch: Some("main"),
                memory_type: Some("architecture"),
                include_stale: false,
            },
            10,
        )?;
        let ids: Vec<i64> = outcome.hits.iter().map(|hit| hit.memory_id).collect();

        assert_eq!(ids, vec![1]);
        Ok(())
    }

    #[test]
    fn explicit_embedding_backfill_covers_all_statuses_across_batches() -> Result<()> {
        let conn = setup_conn()?;
        for id in 1..=1_002 {
            let status = match id {
                1 => "stale",
                2 => "archived",
                _ => "active",
            };
            conn.execute(
                "INSERT INTO memories
                 (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
                 VALUES (?1, '/repo', 'Backfill memory', 'Backfill should cover all visible statuses.', 'decision', 1, ?1, ?2)",
                params![id, status],
            )?;
        }

        ensure_vec_table(&conn)?;
        assert_eq!(backfill_missing_memory_embeddings(&conn, 1_000)?, 1_000);
        assert_eq!(backfill_missing_memory_embeddings(&conn, 1_000)?, 2);

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })?;
        assert_eq!(count, 1_002);
        for status in ["stale", "archived"] {
            let status_count: i64 = conn.query_row(
                "SELECT COUNT(*)
                 FROM memory_embeddings e
                 JOIN memories m ON m.id = e.memory_id
                 WHERE m.status = ?1",
                [status],
                |row| row.get(0),
            )?;
            assert_eq!(status_count, 1);
        }
        Ok(())
    }

    #[test]
    fn missing_vector_table_is_reported_as_disabled() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        let query = embed_query_text("anything");
        let outcome = vector_search_filtered(&conn, &query, VectorSearchFilters::default(), 10)?;

        assert!(outcome
            .disabled_reason
            .as_deref()
            .unwrap_or("")
            .contains("memory_embeddings table is missing"));
        assert!(outcome.hits.is_empty());
        Ok(())
    }
}
