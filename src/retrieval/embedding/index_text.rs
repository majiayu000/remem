//! GH-850 authoritative index passage: canonical fields plus the index-only
//! `search_context` snapshot. FTS and the vector channel must consume the
//! same snapshot; this module is the vector side of that contract.

use anyhow::Result;
use sha2::{Digest, Sha256};

use super::local_semantic::LocalEmbeddingInputKind;
use super::{EmbeddingFallbackCache, TextEmbedding};

/// Versioned prefix for the index passage hash schema. Bump together with
/// `memory_index_text` whenever the passage composition changes.
const MEMORY_INDEX_HASH_SCHEMA: &str = "memory-index-v2";

fn memory_index_text(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    search_context: &str,
) -> String {
    let mut text = super::memory_embedding_text(title, content, memory_type, topic_key);
    if !search_context.trim().is_empty() {
        text.push('\n');
        text.push_str(search_context);
    }
    text
}

pub fn embed_memory_index(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    search_context: &str,
) -> Result<TextEmbedding> {
    let text = memory_index_text(title, content, memory_type, topic_key, search_context);
    super::embed_text(&text, LocalEmbeddingInputKind::Passage)
}

pub(crate) fn embed_memory_index_with_fallback_cache(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    search_context: &str,
    cache: &mut EmbeddingFallbackCache,
) -> Result<TextEmbedding> {
    let text = memory_index_text(title, content, memory_type, topic_key, search_context);
    super::embed_text_with_fallback_cache(&text, LocalEmbeddingInputKind::Passage, cache)
}

/// SHA-256 over the versioned index passage schema plus bytes. Stored as
/// `memory_embeddings.content_hash` and mirrored in
/// `memories.search_context_index_hash` once enrichment is ready.
pub fn memory_index_hash(
    title: &str,
    content: &str,
    memory_type: &str,
    topic_key: Option<&str>,
    search_context: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(MEMORY_INDEX_HASH_SCHEMA.as_bytes());
    hasher.update([0]);
    let text = memory_index_text(title, content, memory_type, topic_key, search_context);
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
