//! GH953 stage S1: the injection path must score from `SearchWeights`, not from
//! constants private to `hybrid_context`.

use rusqlite::{params, Connection};

use super::super::hybrid_context::query_hybrid_context_memories_with_weights;
use super::{insert_memory, setup_context_schema};
use crate::retrieval::search::SearchWeights;

const PROJECT: &str = "demo/project";
const EMBEDDING_ENV_KEYS: &[&str] = &[
    "REMEM_CONFIG",
    "REMEM_EMBEDDINGS_PROVIDER",
    "REMEM_EMBEDDING_PROVIDER",
    "REMEM_EMBEDDINGS_FALLBACK",
    "REMEM_EMBEDDINGS_MODEL",
    "REMEM_EMBEDDING_MODEL",
    "REMEM_EMBEDDINGS_DIMENSIONS",
    "REMEM_EMBEDDING_DIMENSIONS",
    "REMEM_EMBEDDINGS_API_KEY",
    "REMEM_EMBEDDING_API_KEY",
    "REMEM_EMBEDDINGS_API_KEY_ENV",
    "REMEM_EMBEDDINGS_BASE_URL",
    "REMEM_EMBEDDING_BASE_URL",
    "REMEM_EMBEDDINGS_TIMEOUT_SECS",
    "REMEM_EMBEDDINGS_MODEL_DIR",
    "OPENAI_API_KEY",
];

struct ScopedFeatureHashEnv {
    _guard: crate::runtime_config::TestEnvGuard,
    saved: Vec<(&'static str, Option<String>)>,
}

impl ScopedFeatureHashEnv {
    fn new() -> Self {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("embedding env lock should acquire");
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

impl Drop for ScopedFeatureHashEnv {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

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

fn ids_for_query(conn: &Connection, query: &str, weights: SearchWeights) -> Vec<i64> {
    query_hybrid_context_memories_with_weights(conn, PROJECT, query, None, &[], 10, weights)
        .expect("injection retrieval should succeed")
        .into_iter()
        .map(|memory| memory.id)
        .collect()
}

fn ids_for(conn: &Connection, weights: SearchWeights) -> Vec<i64> {
    ids_for_query(conn, "postgres connection pooling", weights)
}

fn zero_injection_weights() -> SearchWeights {
    SearchWeights {
        fts: 0.0,
        vector: 0.0,
        entity: 0.0,
        temporal: 0.0,
        fact: 0.0,
        like_fallback: 0.0,
        ..SearchWeights::default()
    }
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
fn fts_and_entity_weights_each_reach_injection() {
    let conn = Connection::open_in_memory().expect("in-memory database");
    seed(&conn);

    let mut fts_only = zero_injection_weights();
    fts_only.fts = 1.0;
    assert_eq!(ids_for(&conn, fts_only).first(), Some(&1));

    let mut entity_only = zero_injection_weights();
    entity_only.entity = 1.0;
    assert_eq!(ids_for(&conn, entity_only).first(), Some(&2));

    assert!(ids_for(&conn, zero_injection_weights()).is_empty());
}

#[test]
fn temporal_weight_reaches_injection() {
    let conn = Connection::open_in_memory().expect("in-memory database");
    setup_context_schema(&conn);
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "opaque historical record",
        "content without the query terms",
        1_700_000_000,
    );
    conn.execute(
        "UPDATE memories SET reference_time_epoch = 1600000000 WHERE id = 1",
        [],
    )
    .expect("reference time update");

    let mut temporal_only = zero_injection_weights();
    temporal_only.temporal = 1.0;
    let query = "what happened on 2020-09-13";
    assert_eq!(
        ids_for_query(&conn, query, temporal_only),
        vec![1],
        "the temporal channel must consume weights.temporal"
    );
    assert!(ids_for_query(&conn, query, zero_injection_weights()).is_empty());
}

#[test]
fn fact_weight_reaches_injection() {
    let conn = Connection::open_in_memory().expect("in-memory database");
    crate::migrate::run_migrations(&conn).expect("migrations");
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "opaque fact source",
        "content without signer terms",
        1_700_000_000,
    );
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, 'HarborMint', 'verified_by', 'Toma Reed', 1690000000, NULL,
                 1700000000, 1, NULL, '[]', 0.95, NULL, 'active', NULL,
                 1700000000, 1700000000)",
        params![PROJECT],
    )
    .expect("memory fact insert");

    let mut fact_only = zero_injection_weights();
    fact_only.fact = 1.0;
    let query = "Who signs HarborMint with Toma Reed?";
    assert_eq!(
        ids_for_query(&conn, query, fact_only),
        vec![1],
        "the fact channel must consume weights.fact"
    );
    assert!(ids_for_query(&conn, query, zero_injection_weights()).is_empty());
}

#[test]
fn like_fallback_weight_reaches_injection() {
    let conn = Connection::open_in_memory().expect("in-memory database");
    setup_context_schema(&conn);
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "decision",
        "xy",
        "short-token fallback",
        1_700_000_000,
    );

    let mut like_only = zero_injection_weights();
    like_only.like_fallback = 1.0;
    assert_eq!(
        ids_for_query(&conn, "xy", like_only),
        vec![1],
        "the fallback channel must consume weights.like_fallback"
    );
    assert!(ids_for_query(&conn, "xy", zero_injection_weights()).is_empty());
}

#[test]
fn vector_weight_and_distance_each_reach_injection() {
    let _env = ScopedFeatureHashEnv::new();
    let conn = Connection::open_in_memory().expect("in-memory database");
    setup_context_schema(&conn);
    insert_memory(
        &conn,
        1,
        PROJECT,
        None,
        "architecture",
        "credential store",
        "SQLCipher encrypts secrets at rest",
        1_700_000_000,
    );
    crate::retrieval::vector::upsert_memory_embedding_for_row(&conn, 1)
        .expect("feature-hash embedding insert");

    let query = "protect private persisted data";
    let mut vector_only = zero_injection_weights();
    vector_only.vector = 1.0;
    vector_only.max_vector_distance = 1.0;
    assert_eq!(
        ids_for_query(&conn, query, vector_only),
        vec![1],
        "the vector channel must consume weights.vector"
    );

    let mut vector_off = vector_only;
    vector_off.vector = 0.0;
    assert!(ids_for_query(&conn, query, vector_off).is_empty());

    let mut distance_closed = vector_only;
    distance_closed.max_vector_distance = -1.0;
    assert!(
        ids_for_query(&conn, query, distance_closed).is_empty(),
        "the vector threshold must consume weights.max_vector_distance"
    );
}

#[test]
fn rrf_k_reaches_injection_fusion() {
    let conn = Connection::open_in_memory().expect("in-memory database");
    setup_context_schema(&conn);
    for (id, title, content, updated_at) in [
        (
            1,
            "postgres pooling postgres pooling",
            "postgres pooling postgres pooling",
            1_700_000_001,
        ),
        (2, "postgres pooling", "secondary text match", 1_700_000_002),
        (
            3,
            "opaque entity leader",
            "no lexical overlap",
            1_700_000_003,
        ),
    ] {
        insert_memory(
            &conn, id, PROJECT, None, "decision", title, content, updated_at,
        );
    }
    conn.execute(
        "INSERT INTO entities (id, canonical_name, entity_type, created_at_epoch)
         VALUES (1, 'postgres', 'technology', 1700000000)",
        [],
    )
    .expect("entity insert");
    conn.execute(
        "INSERT INTO memory_entities (memory_id, entity_id) VALUES (2, 1), (3, 1)",
        [],
    )
    .expect("memory entity insert");

    let mut weights = zero_injection_weights();
    weights.fts = 1.0;
    // Entity is rank-only: it no longer receives a synthetic second rank
    // boost. A calibrated weight in the crossover interval keeps this fixture
    // sensitive to rrf_k under the production scoring contract.
    weights.entity = 1.25;
    weights.rrf_k = 0.0;
    let small_k = ids_for_query(&conn, "postgres pooling", weights);
    weights.rrf_k = 60.0;
    let large_k = ids_for_query(&conn, "postgres pooling", weights);

    assert_eq!(small_k.first(), Some(&1), "small k should favor rank one");
    assert_eq!(
        large_k.first(),
        Some(&2),
        "large k should favor the cross-channel hit"
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
    for field in [
        "weights.fts",
        "weights.vector",
        "weights.entity",
        "weights.temporal",
        "weights.fact",
        "weights.like_fallback",
        "weights.max_vector_distance",
        "weights.rrf_k",
    ] {
        assert!(
            source.contains(field),
            "injection must read {field} directly from SearchWeights"
        );
    }
    assert!(
        !source.contains("rank_normalized_score"),
        "injection rank-only channels must not feed rank back as a score"
    );
    assert!(
        source.contains("WeightedRankedHit::rank_only"),
        "injection must mark channels without calibrated strength as rank-only"
    );
}
