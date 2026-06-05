use anyhow::Result;
use rusqlite::{params, Connection};

use super::support::setup_entity_schema;
use crate::retrieval::entity::{
    expand_via_entity_graph, expand_via_entity_graph_filtered, link_entities,
    refresh_memory_entities, search_by_entity, search_by_entity_filtered,
};

#[test]
fn search_by_entity_fallback_matches_partial_name() {
    let conn = Connection::open_in_memory().unwrap();
    setup_entity_schema(&conn);
    conn.execute(
        "INSERT INTO memories (id, project, memory_type, status) VALUES (?1, ?2, 'discovery', 'active')",
        params![1_i64, "test/proj"],
    )
    .unwrap();
    link_entities(&conn, 1, &["SQLCipher".to_string()]).unwrap();

    let ids = search_by_entity(&conn, "sql", Some("test/proj"), 10).unwrap();
    assert_eq!(ids, vec![1]);
}

#[test]
fn link_entities_does_not_overcount_duplicate_links() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_entity_schema(&conn);
    conn.execute(
        "INSERT INTO memories (id, project, memory_type, status) VALUES (?1, ?2, 'discovery', 'active')",
        params![1_i64, "test/proj"],
    )?;

    link_entities(
        &conn,
        1,
        &[
            "SQLCipher".to_string(),
            "sqlcipher".to_string(),
            " SQLCipher ".to_string(),
        ],
    )?;
    link_entities(&conn, 1, &["SQLCipher".to_string()])?;

    let mention_count: i64 = conn.query_row(
        "SELECT mention_count FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
        params!["SQLCipher"],
        |row| row.get(0),
    )?;
    let link_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_entities WHERE memory_id = ?1",
        params![1_i64],
        |row| row.get(0),
    )?;

    assert_eq!(mention_count, 1);
    assert_eq!(link_count, 1);
    Ok(())
}

#[test]
fn refresh_memory_entities_replaces_obsolete_links_and_counts() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_entity_schema(&conn);
    for id in 1_i64..=2_i64 {
        conn.execute(
            "INSERT INTO memories (id, project, memory_type, status) VALUES (?1, ?2, 'discovery', 'active')",
            params![id, "test/proj"],
        )?;
    }
    link_entities(&conn, 1, &["SQLCipher".to_string(), "Tokio".to_string()])?;
    link_entities(&conn, 2, &["SQLCipher".to_string()])?;

    refresh_memory_entities(&conn, 1, &["Axum".to_string(), "Tokio".to_string()])?;

    assert_eq!(
        search_by_entity(&conn, "SQLCipher", Some("test/proj"), 10)?,
        vec![2]
    );
    assert_eq!(
        search_by_entity(&conn, "Axum", Some("test/proj"), 10)?,
        vec![1]
    );

    let sqlcipher_count: i64 = conn.query_row(
        "SELECT mention_count FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
        params!["SQLCipher"],
        |row| row.get(0),
    )?;
    let axum_count: i64 = conn.query_row(
        "SELECT mention_count FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
        params!["Axum"],
        |row| row.get(0),
    )?;

    assert_eq!(sqlcipher_count, 1);
    assert_eq!(axum_count, 1);
    Ok(())
}

#[test]
fn refresh_memory_entities_with_empty_list_clears_links() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_entity_schema(&conn);
    conn.execute(
        "INSERT INTO memories (id, project, memory_type, status) VALUES (?1, ?2, 'discovery', 'active')",
        params![1_i64, "test/proj"],
    )?;
    link_entities(&conn, 1, &["SQLCipher".to_string()])?;

    refresh_memory_entities(&conn, 1, &[])?;

    assert!(search_by_entity(&conn, "SQLCipher", Some("test/proj"), 10)?.is_empty());
    let mention_count: i64 = conn.query_row(
        "SELECT mention_count FROM entities WHERE canonical_name = ?1 COLLATE NOCASE",
        params!["SQLCipher"],
        |row| row.get(0),
    )?;
    assert_eq!(mention_count, 0);
    Ok(())
}

#[test]
fn expand_via_entity_graph_excludes_seed_and_excluded_ids() {
    let conn = Connection::open_in_memory().unwrap();
    setup_entity_schema(&conn);
    for id in 1_i64..=4_i64 {
        conn.execute(
            "INSERT INTO memories (id, project, memory_type, status) VALUES (?1, ?2, 'discovery', 'active')",
            params![id, "test/proj"],
        )
        .unwrap();
    }
    link_entities(&conn, 1, &["Tom".to_string(), "Lego".to_string()]).unwrap();
    link_entities(&conn, 2, &["Tom".to_string(), "Chess".to_string()]).unwrap();
    link_entities(&conn, 3, &["Lego".to_string()]).unwrap();
    link_entities(&conn, 4, &["Tom".to_string()]).unwrap();

    let ids = expand_via_entity_graph(&conn, &[1], &[4], Some("test/proj"), 10).unwrap();
    assert!(ids.contains(&2));
    assert!(ids.contains(&3));
    assert!(!ids.contains(&1));
    assert!(!ids.contains(&4));
}

#[test]
fn expand_via_entity_graph_filtered_respects_branch_and_status() {
    let conn = Connection::open_in_memory().unwrap();
    setup_entity_schema(&conn);

    for (id, branch, status) in [
        (1_i64, Some("main"), "active"),
        (2_i64, Some("main"), "active"),
        (3_i64, Some("feature"), "active"),
        (4_i64, None, "active"),
        (5_i64, Some("main"), "archived"),
    ] {
        conn.execute(
            "INSERT INTO memories (id, project, memory_type, branch, status) VALUES (?1, ?2, 'discovery', ?3, ?4)",
            params![id, "test/proj", branch, status],
        )
        .unwrap();
    }

    link_entities(&conn, 1, &["Tom".to_string(), "Lego".to_string()]).unwrap();
    link_entities(&conn, 2, &["Tom".to_string()]).unwrap();
    link_entities(&conn, 3, &["Tom".to_string()]).unwrap();
    link_entities(&conn, 4, &["Tom".to_string()]).unwrap();
    link_entities(&conn, 5, &["Tom".to_string()]).unwrap();

    let ids = expand_via_entity_graph_filtered(
        &conn,
        &[1],
        &[],
        Some("test/proj"),
        None,
        Some("main"),
        10,
        false,
    )
    .unwrap();

    assert!(ids.contains(&2));
    assert!(ids.contains(&4));
    assert!(!ids.contains(&1));
    assert!(!ids.contains(&3));
    assert!(!ids.contains(&5));
}

#[test]
fn search_by_entity_project_filter_includes_global_scope_records() {
    let conn = Connection::open_in_memory().unwrap();
    setup_entity_schema(&conn);
    conn.execute(
        "INSERT INTO memories (id, project, memory_type, status, scope) VALUES (?1, ?2, 'decision', 'active', 'project')",
        params![1_i64, "proj"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories (id, project, memory_type, status, scope) VALUES (?1, ?2, 'preference', 'active', 'global')",
        params![2_i64, "other-proj"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memories (id, project, memory_type, status, scope) VALUES (?1, ?2, 'decision', 'active', 'project')",
        params![3_i64, "other-proj"],
    )
    .unwrap();
    link_entities(&conn, 1, &["SQLite".to_string()]).unwrap();
    link_entities(&conn, 2, &["SQLite".to_string()]).unwrap();
    link_entities(&conn, 3, &["SQLite".to_string()]).unwrap();

    let ids = search_by_entity(&conn, "SQLite", Some("proj"), 10).unwrap();
    assert_eq!(ids, vec![1, 2]);
    assert!(!ids.contains(&3));
}

#[test]
fn search_by_entity_ranks_project_scope_before_same_project_global_scope() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_entity_schema(&conn);
    conn.execute(
        "INSERT INTO memories
         (id, project, memory_type, status, scope, updated_at_epoch)
         VALUES (?1, ?2, 'decision', 'active', 'project', ?3)",
        params![1_i64, "proj", 1_i64],
    )?;
    conn.execute(
        "INSERT INTO memories
         (id, project, memory_type, status, scope, updated_at_epoch)
         VALUES (?1, ?2, 'preference', 'active', 'global', ?3)",
        params![2_i64, "proj", 100_i64],
    )?;
    link_entities(&conn, 1, &["SQLite".to_string()])?;
    link_entities(&conn, 2, &["SQLite".to_string()])?;

    let ids = search_by_entity(&conn, "SQLite", Some("proj"), 1)?;

    assert_eq!(ids, vec![1]);
    Ok(())
}

#[test]
fn search_by_entity_filtered_respects_branch_and_status() {
    let conn = Connection::open_in_memory().unwrap();
    setup_entity_schema(&conn);

    for (id, branch, status) in [
        (1_i64, Some("main"), "active"),
        (2_i64, Some("main"), "active"),
        (3_i64, Some("feature"), "active"),
        (4_i64, None, "active"),
        (5_i64, Some("main"), "archived"),
    ] {
        conn.execute(
            "INSERT INTO memories (id, project, memory_type, branch, status) VALUES (?1, ?2, 'discovery', ?3, ?4)",
            params![id, "test/proj", branch, status],
        )
        .unwrap();
        link_entities(&conn, id, &["Tom".to_string()]).unwrap();
    }

    let ids = search_by_entity_filtered(
        &conn,
        "Tom",
        Some("test/proj"),
        None,
        Some("main"),
        10,
        false,
    )
    .unwrap();

    assert!(ids.contains(&1));
    assert!(ids.contains(&2));
    assert!(ids.contains(&4));
    assert!(!ids.contains(&3));
    assert!(!ids.contains(&5));
}
