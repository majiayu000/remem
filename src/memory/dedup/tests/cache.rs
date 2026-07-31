use anyhow::Result;
use rusqlite::Connection;

use super::{insert_observation, setup_dedup_schema, with_embedding_provider};
use crate::memory::dedup::{check_duplicate, find_hash_duplicates};

#[test]
fn cached_hash_query_rebinds_project() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_dedup_schema(&conn)?;

    let project_a = insert_observation(&conn, "project-a", "Project A exact observation")?;
    let project_b = insert_observation(&conn, "project-b", "Project B exact observation")?;
    let hash_a = crate::db::content_identity_hash("Project A exact observation".as_bytes());
    let hash_b = crate::db::content_identity_hash("Project B exact observation".as_bytes());

    assert_eq!(
        find_hash_duplicates(&conn, "project-a", &hash_a, 900)?,
        vec![project_a]
    );
    assert_eq!(
        find_hash_duplicates(&conn, "project-b", &hash_b, 900)?,
        vec![project_b]
    );
    Ok(())
}

#[test]
fn cached_vector_query_rebinds_project() -> Result<()> {
    with_embedding_provider("feature-hash", || -> Result<()> {
        let conn = Connection::open_in_memory()?;
        setup_dedup_schema(&conn)?;

        insert_observation(
            &conn,
            "project-a",
            "The release workflow rotates archived changelog entries.",
        )?;
        let unrelated = check_duplicate(
            &conn,
            "project-a",
            "Protect private secrets at rest with encryption.",
            None,
        )?;
        assert_eq!(unrelated, None);

        let expected = insert_observation(
            &conn,
            "project-b",
            "SQLCipher encrypts private secrets at rest.",
        )?;
        let duplicate = check_duplicate(
            &conn,
            "project-b",
            "Protect private secrets at rest with encryption.",
            None,
        )?;
        assert_eq!(duplicate, Some(expected));
        Ok(())
    })
}
