use anyhow::{anyhow, Result};
use rusqlite::Connection;

use super::apply_update;
use crate::memory::insert_memory;
use crate::memory::tests_helper::setup_memory_schema;

#[test]
fn update_writes_replacement_embedding_and_filters_stale_vector_rows() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle-vector";
    let old_id = insert_memory(
        &conn,
        Some("s1"),
        project,
        Some("deploy-target"),
        "Deploy target",
        "Deploy target is staging.",
        "decision",
        None,
    )?;

    let outcome = apply_update(
        &conn,
        Some("s2"),
        project,
        "deploy-target",
        "Deploy target corrected",
        "Deploy target is production.",
        "decision",
        None,
        None,
        "project",
        &[old_id],
    )?;
    let new_id = outcome
        .memory_id
        .ok_or_else(|| anyhow!("update should create replacement"))?;

    let embedding_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_embeddings", [], |row| {
            row.get(0)
        })?;
    assert_eq!(embedding_count, 2);

    let query = crate::retrieval::vector::embed_query_text("production deploy target");
    let active = crate::retrieval::vector::vector_search_filtered(
        &conn,
        &query,
        crate::retrieval::vector::VectorSearchFilters {
            project: Some(project),
            include_stale: false,
            ..crate::retrieval::vector::VectorSearchFilters::default()
        },
        10,
    )?;
    let active_ids: Vec<i64> = active.hits.iter().map(|hit| hit.memory_id).collect();
    assert!(active_ids.contains(&new_id), "{active_ids:?}");
    assert!(!active_ids.contains(&old_id), "{active_ids:?}");

    let with_stale = crate::retrieval::vector::vector_search_filtered(
        &conn,
        &query,
        crate::retrieval::vector::VectorSearchFilters {
            project: Some(project),
            include_stale: true,
            ..crate::retrieval::vector::VectorSearchFilters::default()
        },
        10,
    )?;
    let stale_ids: Vec<i64> = with_stale.hits.iter().map(|hit| hit.memory_id).collect();
    assert!(stale_ids.contains(&new_id), "{stale_ids:?}");
    assert!(stale_ids.contains(&old_id), "{stale_ids:?}");
    Ok(())
}
