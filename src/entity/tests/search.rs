use rusqlite::{params, Connection};

use super::support::setup_entity_schema;
use crate::entity::{
    expand_via_entity_graph, expand_via_entity_graph_filtered, link_entities, search_by_entity,
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
