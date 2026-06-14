use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::search_with_branch_explain;

fn setup_explain_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    Ok(conn)
}

struct ExplainMemory<'a> {
    id: i64,
    project: &'a str,
    title: &'a str,
    content: &'a str,
    scope: &'a str,
    updated_at_epoch: i64,
}

fn insert_explain_memory(conn: &Connection, memory: &ExplainMemory<'_>) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, 'decision', NULL, ?6, ?6, 'active', NULL, ?7)",
        params![
            memory.id,
            format!("session-{}", memory.id),
            memory.project,
            memory.title,
            memory.content,
            memory.updated_at_epoch,
            memory.scope,
        ],
    )?;
    Ok(())
}

#[test]
fn search_explain_reports_channels_scores_and_visibility() -> Result<()> {
    let conn = setup_explain_conn()?;
    let now = chrono::Utc::now().timestamp();
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "Recently SQLite project fix",
            content: "recently SQLite project migration fix",
            scope: "project",
            updated_at_epoch: now - 100,
        },
    )?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 2,
            project: "/elsewhere",
            title: "Recently SQLite global preference",
            content: "recently SQLite global preference",
            scope: "global",
            updated_at_epoch: now - 90,
        },
    )?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 3,
            project: "/repo",
            title: "Recently unrelated note",
            content: "recently unrelated note",
            scope: "project",
            updated_at_epoch: now - 80,
        },
    )?;
    crate::retrieval::entity::link_entities(&conn, 1, &["SQLite".to_string()])?;
    crate::retrieval::entity::link_entities(&conn, 2, &["SQLite".to_string()])?;

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("recently SQLite"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert!(!memories.is_empty());
    for expected in ["fts", "entity", "temporal", "vector", "like_fallback"] {
        assert!(
            explain
                .channels
                .iter()
                .any(|channel| channel.name == expected),
            "{expected} channel missing from {:#?}",
            explain.channels
        );
    }
    assert_eq!(explain.rrf_k, 60.0);
    assert!(explain
        .fts_query
        .as_deref()
        .unwrap_or("")
        .contains("SQLite"));
    assert!(explain.temporal_range.is_some());
    assert!(explain
        .results
        .iter()
        .any(|result| result.visibility == "global-overlay"));
    assert!(explain.results.iter().all(|result| {
        result.staleness.status == "active"
            && result.staleness.age == "fresh"
            && result.staleness.source_anchor == "untracked"
            && result.staleness.label.contains("source_anchor=untracked")
    }));
    let like = explain
        .channels
        .iter()
        .find(|channel| channel.name == "like_fallback")
        .context("like_fallback channel should be reported")?;
    assert!(!like.enabled);
    assert!(like
        .disabled_reason
        .as_deref()
        .unwrap_or("")
        .contains("stronger retrieval channels returned hits"));
    assert!(explain.results.iter().all(|result| {
        result
            .contributions
            .iter()
            .all(|contribution| contribution.channel != "like_fallback")
    }));
    assert!(explain.results.iter().all(|result| {
        !result.contributions.is_empty()
            && result
                .contributions
                .iter()
                .all(|contribution| contribution.score > 0.0)
    }));
    Ok(())
}

#[test]
fn like_fallback_only_participates_when_stronger_channels_are_empty() -> Result<()> {
    let conn = setup_explain_conn()?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "DB schema migration",
            content: "Updated AI model",
            scope: "project",
            updated_at_epoch: 100,
        },
    )?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 2,
            project: "/repo",
            title: "Other topic entirely",
            content: "Nothing relevant",
            scope: "project",
            updated_at_epoch: 90,
        },
    )?;

    let (memories, explain) =
        search_with_branch_explain(&conn, Some("DB"), Some("/repo"), None, 5, 0, false, None)?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(1));
    let like = explain
        .channels
        .iter()
        .find(|channel| channel.name == "like_fallback")
        .context("like_fallback channel should be reported")?;
    assert!(like.enabled, "{like:#?}");
    assert_eq!(like.hits.first().map(|hit| hit.memory_id), Some(1));
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == 1)
        .context("LIKE fallback result should be explained")?;
    assert!(result
        .contributions
        .iter()
        .any(|contribution| contribution.channel == "like_fallback" && contribution.score > 0.0));
    Ok(())
}

#[test]
fn semantic_vector_channel_recalls_paraphrase_without_lexical_overlap() -> Result<()> {
    let conn = setup_explain_conn()?;
    let id = crate::memory::insert_memory(
        &conn,
        Some("s1"),
        "/repo",
        Some("credential-storage"),
        "Credential store",
        "SQLCipher encrypts secrets at rest.",
        "architecture",
        None,
    )?;

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("How do we protect private persisted data?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(id));
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == id)
        .context("expected vector-recalled memory in explain results")?;
    assert!(
        result
            .contributions
            .iter()
            .any(|contribution| contribution.channel == "vector"),
        "{result:#?}"
    );
    Ok(())
}

#[test]
fn search_abstains_when_entity_match_lacks_claim_evidence() -> Result<()> {
    let conn = setup_explain_conn()?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "synthetic/kestrelnook",
            title: "Kestrelnook Nebulalatch Owner",
            content: "NebulaLatch is owned by Team Mica.",
            scope: "project",
            updated_at_epoch: 100,
        },
    )?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 2,
            project: "synthetic/kestrelnook",
            title: "Kestrelnook Nebulalatch Quorum Current",
            content: "current NebulaLatch quorum is 7.",
            scope: "project",
            updated_at_epoch: 90,
        },
    )?;
    for id in [1, 2] {
        crate::retrieval::entity::link_entities(
            &conn,
            id,
            &["KestrelNook".to_string(), "NebulaLatch".to_string()],
        )?;
    }

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("Has Project KestrelNook migrated NebulaLatch to Oracle Cloud?"),
        Some("synthetic/kestrelnook"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert!(memories.is_empty(), "{memories:#?}");
    assert!(
        explain.filtered_result_count > 0,
        "entity/FTS candidates should be filtered by evidence gate: {explain:#?}"
    );
    assert!(explain.claim_terms.iter().any(|term| term == "migrated"));
    Ok(())
}

#[test]
fn evidence_gate_preserves_entity_match_with_supported_claim() -> Result<()> {
    let conn = setup_explain_conn()?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "synthetic/kestrelnook",
            title: "Kestrelnook Nebulalatch Quorum Current",
            content: "current NebulaLatch quorum is 7.",
            scope: "project",
            updated_at_epoch: 100,
        },
    )?;
    crate::retrieval::entity::link_entities(
        &conn,
        1,
        &["KestrelNook".to_string(), "NebulaLatch".to_string()],
    )?;

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("Current NebulaLatch quorum for Project kestrelnook?"),
        Some("synthetic/kestrelnook"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(1));
    assert_eq!(explain.filtered_result_count, 0);
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == 1)
        .context("expected retained result in explain")?;
    assert!(result.evidence_confidence >= explain.min_evidence_confidence);
    assert!(explain.claim_terms.iter().any(|term| term == "quorum"));
    Ok(())
}

#[test]
fn evidence_gate_preserves_family_relation_aliases() -> Result<()> {
    let conn = setup_explain_conn()?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "personal",
            title: "Family update from Melanie",
            content: "Melanie mentioned her son Tom and her daughter Sarah.",
            scope: "project",
            updated_at_epoch: 100,
        },
    )?;
    crate::retrieval::entity::link_entities(
        &conn,
        1,
        &[
            "Melanie".to_string(),
            "Tom".to_string(),
            "Sarah".to_string(),
        ],
    )?;

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("Melanie kids"),
        Some("personal"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(1));
    assert!(explain.claim_terms.iter().any(|term| term == "kids"));
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == 1)
        .context("expected retained family relation result")?;
    assert!(result.evidence_confidence >= explain.min_evidence_confidence);
    Ok(())
}

#[test]
fn fact_channel_recalls_source_memory_without_lexical_overlap() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "Signer fact source",
            content: "Signer details live in the temporal fact layer.",
            scope: "project",
            updated_at_epoch: now - 100,
        },
    )?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 2,
            project: "/repo",
            title: "Stale signer fact source",
            content: "Old signer details live outside the searchable text.",
            scope: "project",
            updated_at_epoch: now - 90,
        },
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'HarborMint', 'verified_by', 'Toma Reed', ?1, ?2, ?3, 1,
                 NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
        params![now - 1_000, now + 1_000, now - 900],
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'HarborMint', 'verified_by', 'Toma Reed', ?1, ?2, ?3, 2,
                 NULL, '[]', 0.95, NULL, 'stale', ?4, ?3, ?3)",
        params![now - 1_000, now + 1_000, now - 800, now - 10],
    )?;

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("Who signs HarborMint with Toma Reed?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(1));
    let fact = explain
        .channels
        .iter()
        .find(|channel| channel.name == "fact")
        .context("fact channel should be reported")?;
    assert!(fact.enabled, "{fact:#?}");
    assert_eq!(fact.hits.first().map(|hit| hit.memory_id), Some(1));
    assert!(!fact.hits.iter().any(|hit| hit.memory_id == 2));
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == 1)
        .context("expected fact-recalled result")?;
    assert!(result
        .contributions
        .iter()
        .any(|contribution| contribution.channel == "fact" && contribution.score > 0.0));
    assert_eq!(explain.filtered_result_count, 0);
    Ok(())
}

#[test]
fn search_explain_reports_disabled_vector_channel_when_table_is_missing() -> Result<()> {
    let conn = setup_explain_conn()?;
    conn.execute("DROP TABLE memory_embeddings", [])?;

    let (_memories, explain) = search_with_branch_explain(
        &conn,
        Some("semantic recall"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;
    let vector = explain
        .channels
        .iter()
        .find(|channel| channel.name == "vector")
        .context("vector channel should be reported")?;

    assert!(!vector.enabled);
    assert!(vector
        .disabled_reason
        .as_deref()
        .unwrap_or("")
        .contains("memory_embeddings table is missing"));
    Ok(())
}
