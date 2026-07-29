use super::*;
use rusqlite::params;

fn confidence_test_weights() -> SearchWeights {
    SearchWeights {
        graph: 0.0,
        fact: 0.0,
        max_vector_distance: -1.0,
        ..SearchWeights::default()
    }
}

fn fact_confidence_test_weights() -> SearchWeights {
    SearchWeights {
        graph: 0.0,
        max_vector_distance: -1.0,
        ..SearchWeights::default()
    }
}

fn insert_scope_fixture_memory(
    conn: &Connection,
    project: &str,
    title: &str,
    text: &str,
    scope: &str,
) -> Result<i64> {
    crate::memory::insert_memory_full(
        conn, None, project, None, title, text, "decision", None, None, scope, None,
    )
}

#[test]
fn ordered_load_keeps_global_overlay_and_drops_foreign_project_scope() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    let local = insert_scope_fixture_memory(&conn, "/repo", "local", "local", "project")?;
    let global = insert_scope_fixture_memory(&conn, "/other", "global", "global", "global")?;
    let foreign = insert_scope_fixture_memory(&conn, "/other", "foreign", "foreign", "project")?;

    let loaded = load_ordered_memories(&conn, &[local, global, foreign], Some("/repo"), false)?;
    assert_eq!(
        loaded.iter().map(|memory| memory.id).collect::<Vec<_>>(),
        vec![local, global]
    );
    Ok(())
}

#[test]
fn project_and_destination_entities_cannot_bind_a_different_subject() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    insert_scope_fixture_memory(
        &conn,
        "synthetic/kestrelnook",
        "KestrelNook NebulaLatch Owner",
        "NebulaLatch is owned by Team Mica.",
        "project",
    )?;
    insert_scope_fixture_memory(
        &conn,
        "synthetic/kestrelnook",
        "KestrelNook Oracle Cloud Migration",
        "Project KestrelNook migrated ArchiveFox to Oracle Cloud.",
        "project",
    )?;

    let result = search_with_query_weights(
        &conn,
        "Has Project KestrelNook migrated NebulaLatch to Oracle Cloud?",
        Some("synthetic/kestrelnook"),
        None,
        5,
        0,
        false,
        None,
        false,
        confidence_test_weights(),
    )?;

    assert!(result.is_empty(), "{result:#?}");
    Ok(())
}

#[test]
fn title_case_predicate_remains_required_claim_evidence() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    insert_scope_fixture_memory(
        &conn,
        "/repo",
        "NebulaLatch Ownership",
        "NebulaLatch is owned by Team Mica.",
        "project",
    )?;

    let result = search_with_query_weights(
        &conn,
        "Who Maintains NebulaLatch?",
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        false,
        confidence_test_weights(),
    )?;

    assert!(result.is_empty(), "{result:#?}");
    Ok(())
}

#[test]
fn handles_predicate_rejects_unrelated_entity_content() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    insert_scope_fixture_memory(
        &conn,
        "/repo",
        "NebulaLatch Storage",
        "NebulaLatch uses SQLite WAL mode.",
        "project",
    )?;

    let result = search_with_query_weights(
        &conn,
        "Who handles NebulaLatch?",
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        false,
        confidence_test_weights(),
    )?;

    assert!(result.is_empty(), "{result:#?}");
    Ok(())
}

#[test]
fn common_title_entities_cannot_bridge_an_unrelated_candidate() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    let anchor = insert_scope_fixture_memory(
        &conn,
        "/repo",
        "Current NebulaLatch Quorum",
        "Current NebulaLatch quorum is 7.",
        "project",
    )?;
    let unrelated = insert_scope_fixture_memory(
        &conn,
        "/repo",
        "Current OtherService Quorum",
        "Current OtherService quorum is 3.",
        "project",
    )?;

    let result = search_with_query_weights(
        &conn,
        "Current NebulaLatch quorum?",
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        false,
        confidence_test_weights(),
    )?;
    let ids = result.iter().map(|memory| memory.id).collect::<Vec<_>>();

    assert!(ids.contains(&anchor), "{result:#?}");
    assert!(!ids.contains(&unrelated), "{result:#?}");
    Ok(())
}

#[test]
fn fact_query_requires_primary_and_secondary_entities_in_the_same_fact() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let expected = insert_scope_fixture_memory(
        &conn,
        "/repo",
        "Opaque source alpha",
        "Details live only in structured storage.",
        "project",
    )?;
    let wrong_subject = insert_scope_fixture_memory(
        &conn,
        "/repo",
        "Opaque source beta",
        "Different details live only in structured storage.",
        "project",
    )?;
    let wrong_object = insert_scope_fixture_memory(
        &conn,
        "/repo",
        "Opaque source gamma",
        "More details live only in structured storage.",
        "project",
    )?;
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, learned_at_epoch, source_memory_id,
          source_event_ids, confidence, status, created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'HarborMint', 'verified_by', 'Toma Reed', ?1, ?2, '[]', 0.9,
                 'active', ?1, ?1),
                ('/repo', 'OtherService', 'verified_by', 'Toma Reed', ?1, ?3, '[]', 0.9,
                 'active', ?1, ?1),
                ('/repo', 'HarborMint', 'verified_by', 'Mira Lane', ?1, ?4, '[]', 0.9,
                 'active', ?1, ?1)",
        params![now - 10, expected, wrong_subject, wrong_object],
    )?;

    for query in [
        "Who verified HarborMint with Toma Reed?",
        "who verified harbormint with toma reed?",
    ] {
        let result = search_with_query_weights(
            &conn,
            query,
            Some("/repo"),
            None,
            5,
            0,
            false,
            None,
            false,
            fact_confidence_test_weights(),
        )?;
        let ids = result.iter().map(|memory| memory.id).collect::<Vec<_>>();

        assert_eq!(ids, vec![expected], "query={query}: {result:#?}");
    }
    Ok(())
}
