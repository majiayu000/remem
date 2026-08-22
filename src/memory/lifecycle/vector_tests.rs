use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};

use super::{apply_invalidate, apply_update, MemoryLifecycleOp};
use crate::memory::insert_memory;
use crate::memory::tests_helper::setup_memory_schema;

const EMBEDDING_ENV_KEYS: &[&str] = &[
    "REMEM_CONFIG",
    "REMEM_EMBEDDINGS_PROVIDER",
    "REMEM_EMBEDDING_PROVIDER",
    "REMEM_EMBEDDINGS_MODEL",
    "REMEM_EMBEDDING_MODEL",
    "REMEM_EMBEDDINGS_DIMENSIONS",
    "REMEM_EMBEDDING_DIMENSIONS",
    "REMEM_EMBEDDINGS_FALLBACK",
    "REMEM_EMBEDDINGS_BASE_URL",
    "REMEM_EMBEDDING_BASE_URL",
    "REMEM_EMBEDDINGS_API_KEY",
    "REMEM_EMBEDDING_API_KEY",
    "REMEM_EMBEDDINGS_API_KEY_ENV",
    "REMEM_EMBEDDINGS_TIMEOUT_SECS",
    "REMEM_EMBEDDINGS_MODEL_DIR",
    "OPENAI_API_KEY",
];

struct ScopedEmbeddingProvider {
    _guard: crate::runtime_config::TestEnvGuard,
    saved: Vec<(&'static str, Option<String>)>,
}

impl ScopedEmbeddingProvider {
    fn feature_hash() -> Self {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let saved = EMBEDDING_ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for key in EMBEDDING_ENV_KEYS {
            unsafe { std::env::remove_var(key) };
        }
        unsafe { std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "feature-hash") };
        Self {
            _guard: guard,
            saved,
        }
    }
}

impl Drop for ScopedEmbeddingProvider {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

#[test]
fn update_writes_replacement_embedding_and_filters_stale_vector_rows() -> Result<()> {
    let _embedding_provider = ScopedEmbeddingProvider::feature_hash();
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

#[test]
fn update_records_boundary_evidence_and_rolls_back_invalid_supersede() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle-boundary";
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

    let error = apply_update(
        &conn,
        Some("s2"),
        project,
        "deploy-target-current",
        "Deploy target corrected",
        "Deploy target is production.",
        "decision",
        None,
        None,
        "project",
        &[old_id, 999_999],
    )
    .expect_err("out-of-route supersede must fail before activation");
    assert!(error.to_string().contains("999999"));
    let activation_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_activation_requests",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(activation_count, 1, "only the old-row seed may activate");

    let outcome = apply_update(
        &conn,
        Some("s2"),
        project,
        "deploy-target-current",
        "Deploy target corrected",
        "Deploy target is production.",
        "decision",
        None,
        None,
        "project",
        &[old_id],
    )?;
    assert_eq!(outcome.op, MemoryLifecycleOp::Update);
    let new_id = outcome.memory_id.context("replacement memory id")?;
    let evidence: (String, String, i64) = conn.query_row(
        "SELECT route_kind, source_operation, result_memory_id
         FROM memory_activation_requests WHERE result_memory_id = ?1",
        [new_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        evidence,
        (
            "rust_api".to_string(),
            "memory_lifecycle_update".to_string(),
            new_id,
        )
    );
    Ok(())
}

#[test]
fn update_does_not_discover_same_topic_from_another_branch() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle-branch-route";
    let main_id = crate::memory::insert_memory_full(
        &conn,
        None,
        project,
        Some("deploy-target"),
        "Main deploy target",
        "Main branch deploy target is staging.",
        "decision",
        None,
        Some("main"),
        "project",
        None,
    )?;

    let outcome = apply_update(
        &conn,
        None,
        project,
        "deploy-target",
        "Feature deploy target",
        "Feature branch deploy target is production.",
        "decision",
        None,
        Some("feature"),
        "project",
        &[],
    )?;
    let feature_id = outcome.memory_id.context("feature replacement memory id")?;
    let rows: Vec<(i64, Option<String>, String)> = {
        let mut stmt = conn
            .prepare("SELECT id, branch, status FROM memories WHERE id IN (?1, ?2) ORDER BY id")?;
        let rows = stmt.query_map(params![main_id, feature_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        crate::db::query::collect_rows(rows)?
    };
    assert_eq!(
        rows,
        vec![
            (main_id, Some("main".to_string()), "active".to_string()),
            (
                feature_id,
                Some("feature".to_string()),
                "active".to_string()
            ),
        ]
    );
    Ok(())
}

#[test]
fn invalidate_remains_atomic_for_mixed_validity_ids() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_memory_schema(&conn);
    let project = "test-lifecycle-invalidate";
    let first_id = insert_memory(
        &conn,
        None,
        project,
        Some("first"),
        "First",
        "First value.",
        "discovery",
        None,
    )?;
    let second_id = insert_memory(
        &conn,
        None,
        project,
        Some("second"),
        "Second",
        "Second value.",
        "discovery",
        None,
    )?;

    apply_invalidate(&conn, project, &[first_id, 999_999, second_id], None)
        .expect_err("mixed-validity invalidation must roll back");
    let active: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE id IN (?1, ?2) AND status = 'active'",
        [first_id, second_id],
        |row| row.get(0),
    )?;
    assert_eq!(active, 2);
    Ok(())
}
