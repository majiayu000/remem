//! GH-850 focused channel test: the vector channel consumes the same
//! index-only `search_context` snapshot as FTS, via the versioned
//! `memory-index-v2` passage and hash.

use rusqlite::params;

use super::*;

#[test]
fn vector_channel_consumes_search_context_snapshot() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    ensure_vec_table(&conn)?;

    let title = "Credential store";
    let content = "SQLCipher encrypts secrets at rest.";
    let plain_hash =
        crate::retrieval::embedding::memory_index_hash(title, content, "architecture", None, "");
    let enriched_context = "context: protects private data\nkeywords: quokkaindexterm";
    let enriched_hash = crate::retrieval::embedding::memory_index_hash(
        title,
        content,
        "architecture",
        None,
        enriched_context,
    );
    assert_ne!(
        plain_hash, enriched_hash,
        "index hash must change with the search_context snapshot"
    );

    upsert_memory_embedding(&conn, 1, title, content, "architecture", None, "")?;
    let stored: String = conn.query_row(
        "SELECT content_hash FROM memory_embeddings WHERE memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(stored, plain_hash);

    upsert_memory_embedding(
        &conn,
        1,
        title,
        content,
        "architecture",
        None,
        enriched_context,
    )?;
    let stored: String = conn.query_row(
        "SELECT content_hash FROM memory_embeddings WHERE memory_id = 1",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        stored, enriched_hash,
        "stored content_hash must equal the versioned index hash of the enriched snapshot"
    );

    // The enrichment-only term shifts the vector: a query for the term must
    // rank the enriched passage closer than the plain passage did.
    let query = embed_query_text("quokkaindexterm");
    let outcome = vector_search_filtered(&conn, &query, VectorSearchFilters::default(), 5)?;
    assert!(outcome.disabled_reason.is_none());
    assert_eq!(outcome.hits.len(), 1);
    let enriched_distance = outcome.hits[0].distance;

    upsert_memory_embedding(&conn, 1, title, content, "architecture", None, "")?;
    let outcome = vector_search_filtered(&conn, &query, VectorSearchFilters::default(), 5)?;
    let plain_distance = outcome.hits[0].distance;
    assert!(
        enriched_distance < plain_distance,
        "enriched passage must be closer to the enrichment-only query \
         (enriched={enriched_distance}, plain={plain_distance})"
    );
    Ok(())
}

/// Raw canonical updates that bypass the production writer drop the stale
/// vector in the same transaction (convergence trigger), so an old-passage
/// vector can never keep matching new canonical bytes.
#[test]
fn raw_canonical_update_drops_stale_vector() -> Result<()> {
    let conn = setup_vector_conn()?;
    insert_test_memory(&conn, 1)?;
    ensure_vec_table(&conn)?;
    upsert_memory_embedding(
        &conn,
        1,
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
        "",
    )?;
    assert_eq!(embedding_count(&conn)?, 1);

    conn.execute(
        "UPDATE memories SET content = 'completely different canonical text' WHERE id = 1",
        params![],
    )?;
    assert_eq!(
        embedding_count(&conn)?,
        0,
        "bypass canonical update must remove the stale vector immediately"
    );
    Ok(())
}
