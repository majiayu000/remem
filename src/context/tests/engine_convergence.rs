//! GH953 stage S1: the injection path must score from `SearchWeights`, not from
//! constants private to `hybrid_context`.

use rusqlite::Connection;

use super::super::hybrid_context::query_hybrid_context_memories_with_weights;
use super::{insert_memory, setup_context_schema};
use crate::retrieval::search::SearchWeights;

const PROJECT: &str = "demo/project";

/// Two memories that each win a different channel. Memory 1 matches the query
/// text, so it wins FTS. Memory 2 shares no query terms but is linked to the
/// `postgres` entity, so it wins the entity channel. Which one ranks first is
/// then purely a weight decision.
fn seed(conn: &Connection) {
    setup_context_schema(conn);
    insert_memory(
        conn,
        1,
        PROJECT,
        None,
        "decision",
        "postgres connection pooling",
        "postgres connection pooling was chosen for the write path",
        1_710_000_000,
    );
    insert_memory(
        conn,
        2,
        PROJECT,
        None,
        "decision",
        "unrelated title",
        "body sharing no query terms at all",
        1_710_000_100,
    );
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, created_at_epoch)
         VALUES (1, 'postgres', 'technology', 1710000000)",
        [],
    )
    .expect("entity insert");
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (2, 1)",
        [],
    )
    .expect("memory_entities insert");
}

fn ids_for(conn: &Connection, weights: SearchWeights) -> Vec<i64> {
    query_hybrid_context_memories_with_weights(
        conn,
        PROJECT,
        "postgres connection pooling",
        None,
        &[],
        10,
        weights,
    )
    .expect("injection retrieval should succeed")
    .into_iter()
    .map(|memory| memory.id)
    .collect()
}

#[test]
fn injection_ordering_follows_search_weights() {
    let conn = Connection::open_in_memory().expect("in-memory database");
    seed(&conn);

    let fts_heavy = SearchWeights {
        fts: 100.0,
        entity: 0.001,
        ..SearchWeights::default()
    };
    let entity_heavy = SearchWeights {
        fts: 0.001,
        entity: 100.0,
        ..SearchWeights::default()
    };

    let fts_first = ids_for(&conn, fts_heavy);
    let entity_first = ids_for(&conn, entity_heavy);

    // The S1 prerequisite for GH953: an explicit SearchWeights value must be
    // observable on the path users actually receive. Before this change
    // hybrid_context read private constants and both runs returned the same
    // order. Making eval-weight-grid execute/apply this path is later work.
    assert_ne!(
        fts_first, entity_first,
        "injection ordering must respond to SearchWeights; got {fts_first:?} for both"
    );
    assert_eq!(
        fts_first.first(),
        Some(&1),
        "fts-weighted run should surface the text match first"
    );
    assert_eq!(
        entity_first.first(),
        Some(&2),
        "entity-weighted run should surface the topic-key match first"
    );
}

#[test]
fn default_weights_are_the_production_path() {
    let conn = Connection::open_in_memory().expect("in-memory database");
    seed(&conn);

    let explicit = ids_for(&conn, SearchWeights::default());
    let production = super::super::hybrid_context::query_hybrid_context_memories(
        &conn,
        PROJECT,
        "postgres connection pooling",
        None,
        &[],
        10,
    )
    .expect("injection retrieval should succeed")
    .into_iter()
    .map(|memory| memory.id)
    .collect::<Vec<_>>();

    assert_eq!(
        explicit, production,
        "the zero-argument entry point must be SearchWeights::default()"
    );
}

/// Guards against the drift returning: a future edit that reintroduces a
/// weight constant in `hybrid_context.rs` re-forks the two engines silently.
#[test]
fn hybrid_context_declares_no_private_scoring_constants() {
    let source = include_str!("../hybrid_context.rs");
    for forbidden in [
        "const RRF_K",
        "const MAX_VECTOR_DISTANCE",
        "const FTS_WEIGHT",
        "const VECTOR_WEIGHT",
        "const ENTITY_WEIGHT",
        "const TEMPORAL_WEIGHT",
        "const FACT_WEIGHT",
        "const LIKE_FALLBACK_WEIGHT",
    ] {
        assert!(
            !source.contains(forbidden),
            "{forbidden} must live in SearchWeights, not in hybrid_context.rs"
        );
    }
}
