use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::{
    search_with_branch_explain, search_with_branch_explain_details_with_suppressed_policy,
    search_with_branch_weights, SearchExplainResult, SearchWeights,
};

mod fact_channel;
mod query_boundaries;

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

fn assert_score_identity(result: &SearchExplainResult) {
    let contribution_sum = result
        .contributions
        .iter()
        .map(|contribution| contribution.score)
        .sum::<f64>();
    assert!(
        (result.fusion_score() - contribution_sum).abs() < 1e-12,
        "{result:#?}"
    );
    let post_fusion_score_factor = result
        .post_fusion_score_factor()
        .expect("production explain results must have a positive fusion score");
    assert!(
        (result.final_score - result.fusion_score() * post_fusion_score_factor).abs() < 1e-12,
        "{result:#?}"
    );
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

    let (memories, explain_details) = search_with_branch_explain_details_with_suppressed_policy(
        &conn,
        Some("recently SQLite"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        false,
    )?;
    let explain_details = explain_details.context("query explain should be present")?;
    let explain = &explain_details.explain;

    assert!(!memories.is_empty());
    for expected in "fts entity temporal vector graph_traversal like_fallback".split_whitespace() {
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
    for result in &explain.results {
        assert_eq!(result.post_fusion_score_factor(), Some(1.0));
        assert_score_identity(result);
    }
    assert_eq!(
        explain_details
            .contribution_breakdowns
            .iter()
            .map(|breakdown| breakdown.memory_id)
            .collect::<Vec<_>>(),
        explain
            .results
            .iter()
            .map(|result| result.memory_id)
            .collect::<Vec<_>>(),
        "breakdowns must have one uniquely associated entry in result order"
    );
    for (result, breakdown) in explain
        .results
        .iter()
        .zip(&explain_details.contribution_breakdowns)
    {
        assert_eq!(breakdown.memory_id, result.memory_id);
        assert_eq!(breakdown.contributions.len(), result.contributions.len());
        for (total, details) in result.contributions.iter().zip(&breakdown.contributions) {
            assert_eq!(
                (details.channel.as_str(), details.rank),
                (total.channel.as_str(), total.rank)
            );
            let signal_factor = 1.0 + details.normalized_signal.unwrap_or(0.0);
            assert_eq!(
                details.total_score,
                details.weight * details.reciprocal_rank * signal_factor
            );
            assert_eq!(details.total_score, total.score);
        }
    }
    Ok(())
}

#[test]
fn search_explain_accounts_for_source_anchor_demotion() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES (1, 'stale-session', 'proj', 'stale-topic', 'Stale marker',
                 'stale marker retrieval', 'decision', '[\"src/stale.rs\"]',
                 100, 100, 'active', 'main', 'project')",
        [],
    )?;
    conn.execute(
        "INSERT INTO git_commits
         (id, project, repo_path, sha, short_sha, branch, message,
          authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
         VALUES (1, 'proj', '/repo', 'source', 'source', 'main', NULL,
                 100, '[\"src/stale.rs\"]', 100, 100)",
        [],
    )?;
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (1, 'content-1', 'stale-session', 'test', 100)",
        [],
    )?;
    conn.execute(
        "INSERT INTO git_commits
         (id, project, repo_path, sha, short_sha, branch, message,
          authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
         VALUES (2, 'proj', '/repo', 'later', 'later', 'main', NULL,
                 200, '[\"src/stale.rs\"]', 200, 200)",
        [],
    )?;

    let (_, explain) = search_with_branch_explain(
        &conn,
        Some("stale marker"),
        Some("proj"),
        None,
        5,
        0,
        false,
        Some("main"),
    )?;
    let explain = explain.context("query explain should be present")?;
    let result = explain
        .results
        .iter()
        .find(|result| result.memory_id == 1)
        .context("stale memory should remain visible after demotion")?;

    assert_eq!(result.staleness.source_anchor, "verify-before-trust");
    assert!(
        (result
            .post_fusion_score_factor()
            .expect("demoted result must retain a positive fusion score")
            - 0.25)
            .abs()
            < 1e-12
    );
    assert_score_identity(result);
    Ok(())
}

#[test]
fn parsed_temporal_number_does_not_filter_temporal_results() -> Result<()> {
    let conn = setup_explain_conn()?;
    let now = chrono::Utc::now().timestamp();
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "Deployment update",
            content: "Deployment configuration changed.",
            scope: "project",
            updated_at_epoch: now - 60,
        },
    )?;

    let (memories, explain) = search_with_branch_explain(
        &conn,
        Some("What changed in the last 30 days?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
    )?;
    let explain = explain.context("query explain should be present")?;

    assert_eq!(memories.first().map(|memory| memory.id), Some(1));
    assert!(explain.temporal_range.is_some());
    assert_eq!(explain.claim_terms, vec!["changed"]);
    Ok(())
}

#[test]
fn cjk_temporal_scaffolding_is_absent_from_search_claims() -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    let exact_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 4)
        .context("exact date should construct")?
        .and_hms_opt(12, 0, 0)
        .context("exact time should construct")?
        .and_utc()
        .timestamp();
    for (query, memory_epoch, expected, days) in [
        ("今天有什么变化", now, vec!["变化"], 0),
        ("2026年5月4日有什么变化", exact_date, vec!["变化"], 0),
        ("最近30天有什么变化", now, vec!["变化"], 30),
        (
            "服务42于30天前有什么变化？",
            now - 29 * 86_400 - 43_200,
            vec!["服务", "42", "变化"],
            30,
        ),
        ("最近的天气", now, vec!["天气"], 3),
        ("最近每天", now, vec!["每天"], 3),
        ("最近的部署每天", now, vec!["部署", "每天"], 3),
    ] {
        let conn = setup_explain_conn()?;
        insert_explain_memory(
            &conn,
            &ExplainMemory {
                id: 1,
                project: "/repo",
                title: "服务42部署变化 天气 每天",
                content: "配置已更新。",
                scope: "project",
                updated_at_epoch: memory_epoch,
            },
        )?;
        let before_epoch = chrono::Utc::now().timestamp();
        let (memories, explain) =
            search_with_branch_explain(&conn, Some(query), Some("/repo"), None, 5, 0, false, None)?;
        let after_epoch = chrono::Utc::now().timestamp();
        let explain = explain.context("query explain should be present")?;
        assert_eq!(memories.first().map(|memory| memory.id), Some(1), "{query}");
        assert_eq!(explain.claim_terms, expected, "{query}");
        if days > 0 {
            let (start, _) = explain
                .temporal_range
                .context("recent query should expose its range")?;
            assert!((before_epoch - days * 86_400..=after_epoch - days * 86_400).contains(&start));
        }
    }
    Ok(())
}

#[test]
fn cjk_temporal_introducers_do_not_compact_with_entity_numbers() {
    for introducer in ["在", "于", "自", "从", "至", "到", "截至", "截止", "自从"] {
        let query = format!("服务42{introducer}7天前有什么变化？");
        let claims = super::claim::query_claim_terms(&query, Some("/repo"), &[]);
        let compact = format!("42{introducer}");
        assert!(
            claims.iter().any(|term| term == "42"),
            "{query}: {claims:?}"
        );
        assert!(
            !claims.iter().any(|term| term == &compact),
            "temporal introducer must not become an entity claim: {query}: {claims:?}"
        );
        assert!(
            !claims.iter().any(|term| term == introducer),
            "temporal introducer must not remain an independent claim: {query}: {claims:?}"
        );
    }

    for query in [
        "截至 2026年5月4日有什么变化？",
        "截止 今天有什么变化？",
        "服务42自从 上周有什么变化？",
        "截至 最近7天有什么变化？",
        "截止 最近有什么变化？",
        "截至 7天前有什么变化？",
    ] {
        let claims = super::claim::query_claim_terms(query, Some("/repo"), &[]);
        assert!(
            !claims
                .iter()
                .any(|term| ["截至", "截止", "自从"].contains(&term.as_str())),
            "spaced temporal introducer must not remain a claim: {query}: {claims:?}"
        );
    }
}

#[test]
fn non_temporal_number_remains_a_required_claim_in_temporal_query() -> Result<()> {
    let conn = setup_explain_conn()?;
    let now = chrono::Utc::now().timestamp();
    for (id, content) in [(1, "Service 42 changed."), (2, "Incident 17 changed.")] {
        insert_explain_memory(
            &conn,
            &ExplainMemory {
                id,
                project: "/repo",
                title: "Deployment update",
                content,
                scope: "project",
                updated_at_epoch: now - 60,
            },
        )?;
    }

    for (query, expected_id, expected_claims, days) in [
        (
            "What changed for service 42 in the last 30 days?",
            1,
            vec!["changed", "service", "42"],
            30,
        ),
        ("service 42, 30 days ago", 1, vec!["service", "42"], 30),
        ("service 42 30 days ago", 1, vec!["service", "42"], 30),
        ("incident 17—3 days ago", 2, vec!["incident", "17"], 3),
    ] {
        let before_epoch = chrono::Utc::now().timestamp();
        let (memories, explain) =
            search_with_branch_explain(&conn, Some(query), Some("/repo"), None, 5, 0, false, None)?;
        let after_epoch = chrono::Utc::now().timestamp();
        let explain = explain.context("query explain should be present")?;
        let (start, _) = explain
            .temporal_range
            .context("temporal range should parse")?;
        assert_eq!(memories.first().map(|memory| memory.id), Some(expected_id));
        assert_eq!(explain.claim_terms, expected_claims, "{query}");
        assert!((before_epoch - days * 86_400..=after_epoch - days * 86_400).contains(&start));
    }
    Ok(())
}

#[test]
fn lowercase_short_query_words_are_not_claim_terms() {
    let core_terms = crate::retrieval::query_expand::core_tokens("Who is HarborMint assigned to?");
    let claims = super::claim::claim_terms(&core_terms, Some("/repo"), &["HarborMint".to_string()]);

    assert!(!claims.contains(&"is".to_string()), "{claims:?}");
    assert!(!claims.contains(&"to".to_string()), "{claims:?}");
}

#[test]
fn arbitrary_title_case_predicate_is_not_removed_by_a_static_list() {
    let query = "Who Maintains NebulaLatch?";
    let core_terms = crate::retrieval::query_expand::core_tokens(query);
    let claims =
        super::claim::claim_terms(&core_terms, Some("/repo"), &["NebulaLatch".to_string()]);

    assert_eq!(claims, vec!["maintains"]);
    assert!(super::claim::has_distinctive_entity_shape("NebulaLatch"));
    assert!(super::claim::has_distinctive_entity_shape("incident-17"));
    assert!(!super::claim::has_distinctive_entity_shape("Maintains"));
    assert!(!super::claim::has_distinctive_entity_shape("Current"));
    assert!(super::claim::claim_term_matches(
        "NebulaLatch is owned by Team Mica",
        "owning"
    ));
    let terms = super::claim::entity_scope_candidates(
        "How can I remember capitalization rules?",
        Some("/repo"),
    );
    assert!(!terms.iter().any(|term| term.eq_ignore_ascii_case("remem")));
    assert!(!terms.iter().any(|term| term.eq_ignore_ascii_case("api")));
}

#[test]
fn short_uppercase_qualifiers_remain_claim_terms() {
    let query = "Who verified HarborMint in EU with R2?";
    let core_terms = crate::retrieval::query_expand::core_tokens(query);
    let claims = super::claim::claim_terms(&core_terms, Some("/repo"), &["HarborMint".to_string()]);

    assert!(claims.contains(&"eu".to_string()), "{claims:?}");
    assert!(claims.contains(&"r2".to_string()), "{claims:?}");
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
    let contribution = result
        .contributions
        .iter()
        .find(|contribution| contribution.channel == "like_fallback")
        .context("LIKE fallback contribution should be explained")?;
    let expected = SearchWeights::default().like_fallback
        / (SearchWeights::default().rrf_k + contribution.rank as f64);
    assert!((contribution.score - expected).abs() < 1e-12);
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
    let vector = result
        .contributions
        .iter()
        .find(|contribution| contribution.channel == "vector")
        .context("vector contribution should be explained")?;
    let pure_rrf =
        SearchWeights::default().vector / (SearchWeights::default().rrf_k + vector.rank as f64);
    assert!(vector.score > pure_rrf, "{result:#?}");
    Ok(())
}

#[test]
fn usage_weight_preserves_vector_only_confidence_gate() -> Result<()> {
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
    conn.execute(
        "UPDATE memories
         SET access_count = 8,
             last_accessed_epoch = ?1
         WHERE id = ?2",
        params![chrono::Utc::now().timestamp(), id],
    )?;

    let memories = search_with_branch_weights(
        &conn,
        Some("How do we protect private persisted data?"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        SearchWeights {
            usage: 1.0,
            ..SearchWeights::default()
        },
    )?;

    assert!(
        memories.iter().any(|memory| memory.id == id),
        "usage must not make vector-only evidence fail the confidence gate: {memories:#?}"
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
fn usage_weight_reranks_only_retrieved_candidates() -> Result<()> {
    let conn = setup_explain_conn()?;
    let now = chrono::Utc::now().timestamp();
    for memory in [
        ExplainMemory {
            id: 1,
            project: "/repo",
            title: "SQLite timeout old path",
            content: "SQLite timeout fix should update busy_timeout.",
            scope: "project",
            updated_at_epoch: now - 100,
        },
        ExplainMemory {
            id: 2,
            project: "/repo",
            title: "SQLite timeout proven path",
            content: "SQLite timeout fix should update busy_timeout.",
            scope: "project",
            updated_at_epoch: now - 90,
        },
        ExplainMemory {
            id: 3,
            project: "/repo",
            title: "Popular unrelated note",
            content: "Unrelated launch checklist for release paperwork.",
            scope: "project",
            updated_at_epoch: now - 80,
        },
    ] {
        insert_explain_memory(&conn, &memory)?;
    }
    conn.execute(
        "UPDATE memories
         SET access_count = CASE id WHEN 1 THEN 1 WHEN 2 THEN 25 WHEN 3 THEN 100 END,
             last_accessed_epoch = CASE id WHEN 1 THEN ?1 WHEN 2 THEN ?2 WHEN 3 THEN ?2 END
         WHERE id IN (1, 2, 3)",
        params![now - 90 * 86_400, now],
    )?;

    let ranked = search_with_branch_weights(
        &conn,
        Some("SQLite timeout busy_timeout"),
        Some("/repo"),
        None,
        5,
        0,
        false,
        None,
        SearchWeights {
            usage: 10.0,
            max_vector_distance: 0.0,
            min_evidence_confidence: 0.0,
            ..SearchWeights::default()
        },
    )?;

    assert_eq!(ranked.first().map(|memory| memory.id), Some(2));
    assert!(
        !ranked.iter().any(|memory| memory.id == 3),
        "usage must not retrieve memories absent from text/vector/fact/entity candidates: {ranked:#?}"
    );
    Ok(())
}

#[test]
fn zero_fact_weight_does_not_block_like_fallback() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let now = chrono::Utc::now().timestamp();
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 1,
            project: "/repo",
            title: "Opaque ticket fact",
            content: "Structured ticket detail only.",
            scope: "project",
            updated_at_epoch: now - 100,
        },
    )?;
    insert_explain_memory(
        &conn,
        &ExplainMemory {
            id: 2,
            project: "/repo",
            title: "PR 12 text note",
            content: "PR 12 is documented in searchable text.",
            scope: "project",
            updated_at_epoch: now - 200,
        },
    )?;
    conn.execute(
        "INSERT INTO memory_facts
         (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES ('/repo', 'PR', 'affects_project', '12', ?1, NULL, ?2, 1,
                 NULL, '[]', 0.95, NULL, 'active', NULL, ?2, ?2)",
        params![now - 1_000, now - 900],
    )?;

    let memories = search_with_branch_weights(
        &conn,
        Some("PR 12"),
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

    assert_eq!(memories.first().map(|memory| memory.id), Some(2));
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
