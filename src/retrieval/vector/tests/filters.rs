use rusqlite::params;

use super::*;

#[test]
fn vector_search_respects_filters() -> Result<()> {
    let conn = setup_vector_conn()?;
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
            "",
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
