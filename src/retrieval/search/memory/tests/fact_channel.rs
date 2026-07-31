use super::*;

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
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 3,
            project: "/repo",
            title: "Partial fact source",
            content: "Another active fact source for a different topic.",
            scope: "project",
            updated_at_epoch: now - 80,
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
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'HarborMint', 'verified_by', 'Mira Lane', ?1, ?2, ?3, 3,
                 NULL, '[]', 0.95, NULL, 'active', NULL, ?3, ?3)",
        params![now - 1_000, now + 1_000, now - 700],
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
    assert!(
        memories[0].text.contains("Temporal facts:"),
        "{memories:#?}"
    );
    assert!(memories[0]
        .text
        .contains("HarborMint verified_by Toma Reed"));
    let fact = explain
        .channels
        .iter()
        .find(|channel| channel.name == "fact")
        .context("fact channel should be reported")?;
    assert!(fact.enabled, "{fact:#?}");
    assert_eq!(fact.hits.first().map(|hit| hit.memory_id), Some(1));
    assert!(!fact.hits.iter().any(|hit| hit.memory_id == 2));
    assert!(!fact.hits.iter().any(|hit| hit.memory_id == 3));
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == 1)
        .context("expected fact-recalled result")?;
    let contribution = result
        .contributions
        .iter()
        .find(|contribution| contribution.channel == "fact")
        .context("fact contribution should be explained")?;
    let expected =
        SearchWeights::default().fact / (SearchWeights::default().rrf_k + contribution.rank as f64);
    assert!((contribution.score - expected).abs() < 1e-12);
    assert_eq!(explain.filtered_result_count, 0);
    Ok(())
}

#[test]
fn fact_evidence_survives_when_text_channels_also_match() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "HarborMint signer source",
            content: "Structured fact source without the verifier name.",
            scope: "project",
            updated_at_epoch: now - 100,
        },
    )?;
    crate::retrieval::entity::link_entities(&conn, 1, &["HarborMint".to_string()])?;
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

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("Who verified HarborMint with Toma Reed?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(1));
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == 1)
        .context("fact and text channel result should survive gate")?;
    assert!(result
        .contributions
        .iter()
        .any(|contribution| contribution.channel == "fact"));
    assert_eq!(explain.filtered_result_count, 0);
    Ok(())
}

#[test]
fn zero_fact_weight_disables_fact_only_results() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "Opaque source",
            content: "Details live only in structured facts.",
            scope: "project",
            updated_at_epoch: now - 100,
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

    let disabled = search_with_branch_weights(
        &conn,
        Some("Who signs HarborMint with Toma Reed?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        SearchWeights {
            fact: 0.0,
            max_vector_distance: 0.0,
            min_evidence_confidence: 0.0,
            ..SearchWeights::default()
        },
    )?;
    let enabled = search_with_branch_weights(
        &conn,
        Some("Who signs HarborMint with Toma Reed?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        SearchWeights {
            max_vector_distance: 0.0,
            min_evidence_confidence: 0.0,
            ..SearchWeights::default()
        },
    )?;

    assert!(disabled.is_empty());
    assert_eq!(enabled.first().map(|memory| memory.id), Some(1));
    Ok(())
}
