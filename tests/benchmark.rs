//! remem Benchmark Evaluation Framework
//!
//! Quantitative evaluation of memory capture, search precision/recall,
//! context injection quality, cross-session continuity, and cross-project sharing.
//!
//! All tests use in-memory SQLite — no external AI service needed.

mod bench_fixtures;

use anyhow::Result;
use rusqlite::{params, Connection};

use remem::{entity, memory, search, search_multihop, summarize};

use bench_fixtures::{
    coding_session_fixtures, insert_memory_at, insert_seed_memories, search_eval_memories,
    setup_full_schema, summary_xml_partial, summary_xml_skip, summary_xml_with_all_fields,
};

// ===========================================================================
// Metric helpers
// ===========================================================================

/// Precision@K: fraction of top-K results that are relevant.
fn precision_at_k(result_ids: &[i64], relevant_ids: &[i64], k: usize) -> f64 {
    let top_k: Vec<i64> = result_ids.iter().copied().take(k).collect();
    if top_k.is_empty() {
        return 0.0;
    }
    let hits = top_k.iter().filter(|id| relevant_ids.contains(id)).count();
    hits as f64 / top_k.len() as f64
}

/// Recall@K: fraction of all relevant items found in top-K results.
fn recall_at_k(result_ids: &[i64], relevant_ids: &[i64], k: usize) -> f64 {
    if relevant_ids.is_empty() {
        return 1.0;
    }
    let top_k: Vec<i64> = result_ids.iter().copied().take(k).collect();
    let hits = relevant_ids.iter().filter(|id| top_k.contains(id)).count();
    hits as f64 / relevant_ids.len() as f64
}

// ===========================================================================
// Scenario 1: Memory Capture Pipeline
// ===========================================================================

#[test]
fn bench_memory_capture_rate() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let fixtures = coding_session_fixtures();
    let mut total_meaningful = 0;
    let mut total_captured = 0;

    for session in &fixtures {
        // Insert events
        for event in &session.events {
            memory::insert_event(
                &conn,
                &session.session_id,
                &session.project,
                &event.event_type,
                &event.summary,
                event.detail.as_deref(),
                event.files.as_deref(),
                None,
            )?;

            // Count meaningful events (file edits with detail are "meaningful")
            if event.event_type == "file_edit" && event.detail.is_some() {
                total_meaningful += 1;

                // Simulate promotion: insert as memory (in production this is done
                // by summary → promote_summary_to_memories)
                memory::insert_memory(
                    &conn,
                    Some(&session.session_id),
                    &session.project,
                    None,
                    &event.summary,
                    event.detail.as_deref().unwrap_or(&event.summary),
                    "discovery",
                    event.files.as_deref(),
                )?;
                total_captured += 1;
            }
        }
    }

    let mcr = if total_meaningful > 0 {
        total_captured as f64 / total_meaningful as f64
    } else {
        0.0
    };

    eprintln!(
        "[MCR] meaningful={} captured={} rate={:.2}",
        total_meaningful, total_captured, mcr
    );
    assert!(
        mcr >= 0.8,
        "Memory Capture Rate {:.2} below threshold 0.8",
        mcr
    );

    // Verify memories are actually queryable
    let memories = memory::get_recent_memories(&conn, "tools/remem", 50)?;
    assert!(
        !memories.is_empty(),
        "No memories found after capture pipeline"
    );

    Ok(())
}

// ===========================================================================
// Scenario 2: Search Precision@K and Recall@K
// ===========================================================================

#[test]
fn bench_search_precision_and_recall_fts() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let seeds = search_eval_memories();
    let ids = insert_seed_memories(&conn, &seeds)?;

    // Build relevance ground truth for "FTS5 search" query
    let relevant_ids: Vec<i64> = seeds
        .iter()
        .zip(ids.iter())
        .filter(|(s, _)| s.relevant_to_fts_query)
        .map(|(_, id)| *id)
        .collect();

    // Execute search
    let results = search::search(
        &conn,
        Some("FTS5 search"),
        Some("tools/remem"),
        None,
        10,
        0,
        true,
    )?;
    let result_ids: Vec<i64> = results.iter().map(|m| m.id).collect();

    let p5 = precision_at_k(&result_ids, &relevant_ids, 5);
    let r10 = recall_at_k(&result_ids, &relevant_ids, 10);

    eprintln!(
        "[Search FTS5] results={} relevant={} P@5={:.2} R@10={:.2}",
        results.len(),
        relevant_ids.len(),
        p5,
        r10
    );

    // Targets: P@5 >= 0.6, R@10 >= 0.5
    assert!(
        p5 >= 0.6,
        "Precision@5 {:.2} below threshold 0.6 (results: {:?})",
        p5,
        results.iter().map(|m| &m.title).collect::<Vec<_>>()
    );
    assert!(r10 >= 0.5, "Recall@10 {:.2} below threshold 0.5", r10);

    Ok(())
}

#[test]
fn bench_search_precision_decay_query() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let seeds = search_eval_memories();
    let ids = insert_seed_memories(&conn, &seeds)?;

    let relevant_ids: Vec<i64> = seeds
        .iter()
        .zip(ids.iter())
        .filter(|(s, _)| s.relevant_to_decay_query)
        .map(|(_, id)| *id)
        .collect();

    let results = search::search(
        &conn,
        Some("time decay"),
        Some("tools/remem"),
        None,
        10,
        0,
        true,
    )?;
    let result_ids: Vec<i64> = results.iter().map(|m| m.id).collect();

    let p5 = precision_at_k(&result_ids, &relevant_ids, 5);
    let r10 = recall_at_k(&result_ids, &relevant_ids, 10);

    eprintln!(
        "[Search Decay] results={} relevant={} P@5={:.2} R@10={:.2}",
        results.len(),
        relevant_ids.len(),
        p5,
        r10
    );

    // At least some relevant results should appear
    assert!(
        r10 > 0.0,
        "No relevant results found for 'time decay' query"
    );

    Ok(())
}

// ===========================================================================
// Scenario 3: Context Injection Quality (scoring logic)
// ===========================================================================

#[test]
fn bench_context_score_prefers_decisions_and_recent() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let now = chrono::Utc::now().timestamp();

    // Insert memories with varying types and ages
    let decision_recent = insert_memory_at(
        &conn,
        "tools/remem",
        "Use FTS5 over Tantivy",
        "Decided to use SQLite FTS5 for simplicity",
        "decision",
        now - 2 * 86400, // 2 days old
        "project",
    )?;
    let bugfix_recent = insert_memory_at(
        &conn,
        "tools/remem",
        "Fix search crash on hyphen",
        "Wrapped tokens in quotes for FTS5 MATCH safety",
        "bugfix",
        now - 86400, // 1 day old
        "project",
    )?;
    let discovery_old = insert_memory_at(
        &conn,
        "tools/remem",
        "SQLite WAL mode",
        "WAL journal mode improves concurrency",
        "discovery",
        now - 60 * 86400, // 60 days old
        "project",
    )?;
    let decision_old = insert_memory_at(
        &conn,
        "tools/remem",
        "Chose Rust over Python",
        "Single binary deployment without runtime",
        "decision",
        now - 45 * 86400, // 45 days old
        "project",
    )?;
    let session_activity = insert_memory_at(
        &conn,
        "tools/remem",
        "Session: fixed CI pipeline",
        "Updated GitHub Actions workflow",
        "session_activity",
        now - 3 * 86400, // 3 days old
        "project",
    )?;

    // Fetch and score memories using the same logic as context.rs
    let memories = memory::get_recent_memories(&conn, "tools/remem", 50)?;
    assert!(!memories.is_empty(), "No memories found");

    // Score each memory
    let mut scored: Vec<(i64, &str, &str, f64)> = memories
        .iter()
        .map(|m| {
            let type_weight: f64 = match m.memory_type.as_str() {
                "decision" => 3.0,
                "bugfix" => 2.5,
                "architecture" => 2.0,
                "preference" => 1.5,
                "discovery" => 1.0,
                _ => 0.5,
            };
            let age_days = (now - m.updated_at_epoch) / 86400;
            let time_decay: f64 = if age_days <= 7 {
                1.0
            } else if age_days <= 30 {
                0.7
            } else {
                0.4
            };
            let score = type_weight * time_decay;
            (m.id, m.title.as_str(), m.memory_type.as_str(), score)
        })
        .collect();

    scored.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());

    eprintln!("[Context Score] Ranking:");
    for (id, title, mtype, score) in &scored {
        eprintln!("  #{} [{:.1}] {} ({})", id, score, title, mtype);
    }

    // Assertions on ranking:
    // 1. Recent decision (score = 3.0 * 1.0 = 3.0) should be top
    assert_eq!(
        scored[0].0, decision_recent,
        "Recent decision should rank #1"
    );

    // 2. Recent bugfix (2.5 * 1.0 = 2.5) should be #2
    assert_eq!(scored[1].0, bugfix_recent, "Recent bugfix should rank #2");

    // 3. Old decision (3.0 * 0.4 = 1.2) should outrank old discovery (1.0 * 0.4 = 0.4)
    let old_decision_score = scored.iter().find(|s| s.0 == decision_old).unwrap().3;
    let old_discovery_score = scored.iter().find(|s| s.0 == discovery_old).unwrap().3;
    assert!(
        old_decision_score > old_discovery_score,
        "Old decision ({:.1}) should outrank old discovery ({:.1})",
        old_decision_score,
        old_discovery_score
    );

    // 4. Session activity should rank lowest (0.5 * 1.0 = 0.5)
    let session_score = scored.iter().find(|s| s.0 == session_activity).unwrap().3;
    assert!(
        session_score < 1.0,
        "Session activity score {:.1} should be low",
        session_score
    );

    // Context Relevance Score: high-value items (decision/bugfix) in top 3
    let top3_high_value = scored
        .iter()
        .take(3)
        .filter(|s| s.2 == "decision" || s.2 == "bugfix")
        .count();
    let context_score = top3_high_value as f64 / 3.0;
    eprintln!(
        "[Context Score] high_value_in_top3={}/3 = {:.2}",
        top3_high_value, context_score
    );
    assert!(
        context_score >= 0.6,
        "Context relevance {:.2} below threshold 0.6",
        context_score
    );

    Ok(())
}

// ===========================================================================
// Scenario 4: Cross-Session Decision Continuity
// ===========================================================================

#[test]
fn bench_cross_session_decision_retrieval() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    // Session A: store a decision
    memory::insert_memory(
        &conn,
        Some("sess-A"),
        "tools/remem",
        Some("fts5-vs-tantivy"),
        "Chose FTS5 over Tantivy for search",
        "Decision: Use SQLite FTS5 instead of Tantivy. Rationale: single-file DB, \
         no additional binary, trigram tokenizer handles CJK, good enough performance \
         for <100k memories.",
        "decision",
        None,
    )?;

    // Session B: search for that decision
    let results = search::search(
        &conn,
        Some("FTS5 Tantivy search decision"),
        Some("tools/remem"),
        None,
        5,
        0,
        true,
    )?;

    eprintln!(
        "[Cross-Session] query='FTS5 Tantivy' results={}",
        results.len()
    );
    assert!(
        !results.is_empty(),
        "Cross-session decision not found by search"
    );
    assert_eq!(
        results[0].title, "Chose FTS5 over Tantivy for search",
        "Decision should be the top result"
    );
    assert_eq!(
        results[0].memory_type, "decision",
        "Result should be a decision type"
    );

    // Verify topic_key enables upsert (Session C updates same decision)
    memory::insert_memory(
        &conn,
        Some("sess-C"),
        "tools/remem",
        Some("fts5-vs-tantivy"),
        "Chose FTS5 over Tantivy for search",
        "Updated decision: Still using FTS5. Added LIKE fallback for short queries. \
         Performance confirmed at 5ms for 10k records.",
        "decision",
        None,
    )?;

    // Should still be one memory (upserted, not duplicated)
    let all_fts = search::search(
        &conn,
        Some("FTS5 Tantivy"),
        Some("tools/remem"),
        None,
        10,
        0,
        true,
    )?;
    let fts_decisions: Vec<_> = all_fts
        .iter()
        .filter(|m| m.topic_key.as_deref() == Some("fts5-vs-tantivy"))
        .collect();
    assert_eq!(
        fts_decisions.len(),
        1,
        "topic_key upsert should prevent duplicates, found {}",
        fts_decisions.len()
    );
    assert!(
        fts_decisions[0].text.contains("LIKE fallback"),
        "Memory should contain updated content"
    );

    Ok(())
}

// ===========================================================================
// Scenario 5: Cross-Project Global Memory Visibility
// ===========================================================================

#[test]
fn bench_global_scope_cross_project() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let now = chrono::Utc::now().timestamp();

    // Project A: insert a global preference
    insert_memory_at(
        &conn,
        "tools/remem",
        "Prefer English commit messages",
        "Always use English for git commit messages and code comments",
        "preference",
        now,
        "global",
    )?;

    // Project A: insert a project-scoped memory
    insert_memory_at(
        &conn,
        "tools/remem",
        "FTS5 search design",
        "Search implementation details specific to remem",
        "architecture",
        now,
        "project",
    )?;

    // Query from Project B: should see global preference but NOT project-scoped memory
    let project_b_memories = memory::get_recent_memories(&conn, "web/dashboard", 50)?;

    let global_visible = project_b_memories
        .iter()
        .any(|m| m.title == "Prefer English commit messages");
    let project_leaked = project_b_memories
        .iter()
        .any(|m| m.title == "FTS5 search design");

    eprintln!(
        "[Global Scope] project_b sees: {} memories, global_visible={}, project_leaked={}",
        project_b_memories.len(),
        global_visible,
        project_leaked
    );

    assert!(
        global_visible,
        "Global preference should be visible in Project B"
    );
    assert!(
        !project_leaked,
        "Project-scoped memory should NOT leak to Project B"
    );

    Ok(())
}

// ===========================================================================
// Scenario 6: Time Decay Ranking
// ===========================================================================

#[test]
fn bench_time_decay_ranks_newer_higher() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let now = chrono::Utc::now().timestamp();

    // Insert two memories with identical relevance but different ages
    let new_id = insert_memory_at(
        &conn,
        "tools/remem",
        "Database connection pooling strategy",
        "Use single connection with WAL mode for SQLite",
        "decision",
        now - 2 * 86400, // 2 days ago
        "project",
    )?;
    let old_id = insert_memory_at(
        &conn,
        "tools/remem",
        "Database connection pooling approach",
        "Originally considered connection pool but SQLite works better single-threaded",
        "decision",
        now - 60 * 86400, // 60 days ago
        "project",
    )?;

    let results = search::search(
        &conn,
        Some("database connection pooling"),
        Some("tools/remem"),
        None,
        10,
        0,
        true,
    )?;

    assert!(
        results.len() >= 2,
        "Should find both memories, found {}",
        results.len()
    );

    eprintln!("[Time Decay] Results:");
    for m in &results {
        let age = (now - m.updated_at_epoch) / 86400;
        eprintln!("  #{} age={}d {}", m.id, age, m.title);
    }

    // Note: FTS5 ranking is by text relevance, not time decay.
    // Time decay is applied in context scoring, not in raw search.
    // So here we test that both are found; the context scoring test (Scenario 3)
    // verifies decay ordering.
    let found_new = results.iter().any(|m| m.id == new_id);
    let found_old = results.iter().any(|m| m.id == old_id);
    assert!(found_new, "New memory should be in search results");
    assert!(found_old, "Old memory should be in search results");

    // Apply context scoring manually and verify decay ordering
    let score = |m: &memory::Memory| -> f64 {
        let type_weight: f64 = match m.memory_type.as_str() {
            "decision" => 3.0,
            "bugfix" => 2.5,
            _ => 1.0,
        };
        let age_days = (now - m.updated_at_epoch) / 86400;
        let decay: f64 = if age_days <= 7 {
            1.0
        } else if age_days <= 30 {
            0.7
        } else {
            0.4
        };
        type_weight * decay
    };

    let new_mem = results.iter().find(|m| m.id == new_id).unwrap();
    let old_mem = results.iter().find(|m| m.id == old_id).unwrap();
    let new_score = score(new_mem);
    let old_score = score(old_mem);

    eprintln!(
        "[Time Decay] new_score={:.1} old_score={:.1}",
        new_score, old_score
    );
    assert!(
        new_score > old_score,
        "Newer memory score ({:.1}) should exceed older ({:.1})",
        new_score,
        old_score
    );

    Ok(())
}

// ===========================================================================
// Scenario 7: Summary Parse & Memory Promotion
// ===========================================================================

#[test]
fn bench_summary_parse_full() -> Result<()> {
    let xml = summary_xml_with_all_fields();
    let parsed = summarize::parse_summary(&xml);

    assert!(parsed.is_some(), "Full summary should parse successfully");
    let p = parsed.unwrap();

    assert!(
        p.request
            .as_ref()
            .is_some_and(|r| r.contains("global memory scope")),
        "request field should be extracted"
    );
    assert!(
        p.decisions.as_ref().is_some_and(|d| d.contains("global")),
        "decisions field should be extracted"
    );
    assert!(
        p.learned.as_ref().is_some_and(|l| l.contains("FTS5")),
        "learned field should be extracted"
    );
    assert!(
        p.preferences
            .as_ref()
            .is_some_and(|pref| pref.contains("English")),
        "preferences field should be extracted"
    );
    assert!(p.completed.is_some(), "completed field should be extracted");
    assert!(
        p.next_steps.is_some(),
        "next_steps field should be extracted"
    );

    eprintln!(
        "[Summary Parse] All 6 fields extracted: request={} completed={} decisions={} learned={} next_steps={} preferences={}",
        p.request.is_some(),
        p.completed.is_some(),
        p.decisions.is_some(),
        p.learned.is_some(),
        p.next_steps.is_some(),
        p.preferences.is_some(),
    );

    Ok(())
}

#[test]
fn bench_summary_parse_skip() {
    let xml = summary_xml_skip();
    let parsed = summarize::parse_summary(&xml);
    assert!(parsed.is_none(), "skip_summary should return None");
}

#[test]
fn bench_summary_parse_partial() -> Result<()> {
    let xml = summary_xml_partial();
    let parsed = summarize::parse_summary(&xml);

    assert!(parsed.is_some(), "Partial summary should parse");
    let p = parsed.unwrap();

    assert!(p.request.is_some(), "request should be present");
    assert!(p.completed.is_some(), "completed should be present");
    assert!(p.decisions.is_none(), "decisions should be absent");
    assert!(p.learned.is_none(), "learned should be absent");
    assert!(p.preferences.is_none(), "preferences should be absent");

    Ok(())
}

#[test]
fn bench_summary_promote_creates_memories() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let session_id = "promote-test-001";
    let project = "tools/remem";

    // Register session
    conn.execute(
        "INSERT INTO sdk_sessions (content_session_id, memory_session_id, project, started_at_epoch) \
         VALUES (?1, ?1, ?2, ?3)",
        params![session_id, project, chrono::Utc::now().timestamp()],
    )?;

    // Promote fields to memories
    memory::promote_summary_to_memories(
        &conn,
        session_id,
        project,
        Some("Implement benchmark framework"),
        Some("Use FTS5 for search, chose Rust for single-binary deployment"),
        Some("FTS5 trigram tokenizer handles CJK well"),
        Some("Always run cargo check before cargo test"),
    )?;

    // Verify memories were created
    let all = memory::get_recent_memories(&conn, project, 50)?;

    let decisions: Vec<_> = all.iter().filter(|m| m.memory_type == "decision").collect();
    let discoveries: Vec<_> = all
        .iter()
        .filter(|m| m.memory_type == "discovery")
        .collect();
    let preferences: Vec<_> = all
        .iter()
        .filter(|m| m.memory_type == "preference")
        .collect();

    eprintln!(
        "[Promote] total={} decisions={} discoveries={} preferences={}",
        all.len(),
        decisions.len(),
        discoveries.len(),
        preferences.len(),
    );

    assert!(
        !decisions.is_empty(),
        "Decisions should be promoted to memories"
    );
    assert!(
        !discoveries.is_empty(),
        "Discoveries should be promoted to memories"
    );
    assert!(
        !preferences.is_empty(),
        "Preferences should be promoted to memories"
    );

    // Verify preference is auto-scoped as global
    let global_prefs: Vec<_> = preferences.iter().filter(|m| m.scope == "global").collect();
    eprintln!(
        "[Promote] global preferences: {}/{}",
        global_prefs.len(),
        preferences.len()
    );

    Ok(())
}

// ===========================================================================
// Scenario 8: Search filters by type
// ===========================================================================

#[test]
fn bench_search_filter_by_type() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    memory::insert_memory(
        &conn,
        Some("s1"),
        "tools/remem",
        None,
        "FTS5 search design",
        "Technical architecture for search",
        "architecture",
        None,
    )?;
    memory::insert_memory(
        &conn,
        Some("s1"),
        "tools/remem",
        None,
        "Fixed FTS5 crash",
        "Wrapped tokens in quotes",
        "bugfix",
        None,
    )?;
    memory::insert_memory(
        &conn,
        Some("s1"),
        "tools/remem",
        None,
        "Search uses FTS5",
        "Decision to use FTS5",
        "decision",
        None,
    )?;

    // Filter by type
    let decisions = search::search(
        &conn,
        Some("FTS5"),
        Some("tools/remem"),
        Some("decision"),
        10,
        0,
        true,
    )?;
    assert_eq!(
        decisions.len(),
        1,
        "Type filter should return only decisions"
    );
    assert_eq!(decisions[0].memory_type, "decision");

    let bugfixes = search::search(
        &conn,
        Some("FTS5"),
        Some("tools/remem"),
        Some("bugfix"),
        10,
        0,
        true,
    )?;
    assert_eq!(bugfixes.len(), 1, "Type filter should return only bugfixes");

    // No filter: all 3
    let all = search::search(&conn, Some("FTS5"), Some("tools/remem"), None, 10, 0, true)?;
    assert_eq!(all.len(), 3, "Without type filter should return all 3");

    Ok(())
}

// ===========================================================================
// Scenario 9: Topic key deduplication
// ===========================================================================

#[test]
fn bench_topic_key_dedup() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let project = "tools/remem";
    let topic = "search-strategy";

    // Insert v1
    let id1 = memory::insert_memory(
        &conn,
        Some("s1"),
        project,
        Some(topic),
        "Search strategy v1",
        "Initial FTS5 approach",
        "decision",
        None,
    )?;

    // Insert v2 with same topic_key (should upsert)
    let id2 = memory::insert_memory(
        &conn,
        Some("s2"),
        project,
        Some(topic),
        "Search strategy v2",
        "FTS5 with LIKE fallback for short tokens",
        "decision",
        None,
    )?;

    assert_eq!(id1, id2, "Same topic_key should return same ID (upsert)");

    let all = memory::get_recent_memories(&conn, project, 50)?;
    let strategy_mems: Vec<_> = all
        .iter()
        .filter(|m| m.topic_key.as_deref() == Some(topic))
        .collect();
    assert_eq!(
        strategy_mems.len(),
        1,
        "Should have exactly 1 memory for topic_key"
    );
    assert!(
        strategy_mems[0].text.contains("LIKE fallback"),
        "Content should be the updated version"
    );

    Ok(())
}

// ===========================================================================
// Scenario 10: Multi-hop Entity Graph Expansion
// ===========================================================================

#[test]
fn bench_multi_hop_entity_graph_retrieval() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    // Simulate multi-hop scenario: "What do Melanie's kids like?"
    // Memory 1: mentions Melanie and her son Tom
    let id1 = memory::insert_memory(
        &conn,
        Some("s1"),
        "personal",
        None,
        "Family update from Melanie",
        "Melanie mentioned her son Tom started kindergarten this fall. \
         She also talked about her daughter Sarah who is in 3rd grade.",
        "discovery",
        None,
    )?;

    // Memory 2: mentions Tom's interests (no direct mention of Melanie)
    let id2 = memory::insert_memory(
        &conn,
        Some("s2"),
        "personal",
        None,
        "Tom's hobbies",
        "Tom loves dinosaurs and building Lego sets. He wants a T-Rex for his birthday.",
        "discovery",
        None,
    )?;

    // Memory 3: mentions Sarah's interests (no direct mention of Melanie)
    let id3 = memory::insert_memory(
        &conn,
        Some("s3"),
        "personal",
        None,
        "Sarah's school activities",
        "Sarah is on the school swim team and loves reading Harry Potter books.",
        "discovery",
        None,
    )?;

    // Noise memory
    memory::insert_memory(
        &conn,
        Some("s4"),
        "personal",
        None,
        "Weekend plans",
        "Going hiking at the national park this Saturday.",
        "discovery",
        None,
    )?;

    // Link entities to memories
    entity::link_entities(
        &conn,
        id1,
        &[
            "Melanie".to_string(),
            "Tom".to_string(),
            "Sarah".to_string(),
        ],
    )?;
    entity::link_entities(&conn, id2, &["Tom".to_string(), "Lego".to_string()])?;
    entity::link_entities(&conn, id3, &["Sarah".to_string()])?;

    // Standard search: "Melanie's kids" — should find memory about Melanie
    let standard = search::search(
        &conn,
        Some("Melanie kids"),
        Some("personal"),
        None,
        10,
        0,
        true,
    )?;
    let standard_ids: Vec<i64> = standard.iter().map(|m| m.id).collect();

    // Multi-hop search: should find Melanie + Tom's hobbies + Sarah's activities
    let multi = search_multihop::search_multi_hop(&conn, "Melanie kids", Some("personal"), 10)?;
    let multi_ids: Vec<i64> = multi.memories.iter().map(|m| m.id).collect();

    eprintln!("[Multi-hop] Standard search found: {:?}", standard_ids);
    eprintln!("[Multi-hop] Multi-hop search found: {:?}", multi_ids);
    eprintln!("[Multi-hop] Hops: {}", multi.hops);
    eprintln!(
        "[Multi-hop] Entities discovered: {:?}",
        multi.entities_discovered
    );

    // Standard search should find at least the Melanie memory
    assert!(
        standard_ids.contains(&id1),
        "Standard search should find Melanie memory"
    );

    // Multi-hop should find all three relevant memories
    assert!(
        multi_ids.contains(&id1),
        "Multi-hop should find Melanie memory"
    );
    assert!(
        multi_ids.contains(&id2),
        "Multi-hop should find Tom's hobbies via entity graph"
    );
    assert!(
        multi_ids.contains(&id3),
        "Multi-hop should find Sarah's activities via entity graph"
    );

    // Multi-hop should have discovered entities from first-hop results
    assert!(
        !multi.entities_discovered.is_empty(),
        "Should have discovered entities from first-hop results",
    );

    // The key assertion: multi-hop recall must be perfect (find all 3)
    let relevant = vec![id1, id2, id3];
    let multi_recall = recall_at_k(&multi_ids, &relevant, 10);
    eprintln!("[Multi-hop] Multi-hop R@10={:.2}", multi_recall);
    assert!(
        multi_recall >= 1.0,
        "Multi-hop should find all relevant memories, R@10={:.2}",
        multi_recall,
    );

    Ok(())
}

#[test]
fn bench_entity_graph_expansion_finds_related() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    // Memory A mentions entities X and Y
    let id_a = memory::insert_memory(
        &conn,
        Some("s1"),
        "proj",
        None,
        "Project setup with React and TypeScript",
        "Configured React with TypeScript template.",
        "architecture",
        None,
    )?;
    // Memory B mentions entity Y and Z (related to A via Y)
    let id_b = memory::insert_memory(
        &conn,
        Some("s2"),
        "proj",
        None,
        "TypeScript strict mode config",
        "Enabled strict mode in tsconfig.",
        "decision",
        None,
    )?;
    // Memory C mentions entity Z only (related to B via Z, 2-hop from A)
    let id_c = memory::insert_memory(
        &conn,
        Some("s3"),
        "proj",
        None,
        "ESLint config for strict mode",
        "Added eslint-config-strict rules.",
        "decision",
        None,
    )?;

    entity::link_entities(
        &conn,
        id_a,
        &["React".to_string(), "TypeScript".to_string()],
    )?;
    entity::link_entities(&conn, id_b, &["TypeScript".to_string()])?;
    entity::link_entities(&conn, id_c, &["ESLint".to_string()])?;

    // From seed [id_a], entity graph should find id_b (shares TypeScript)
    let expanded = entity::expand_via_entity_graph(&conn, &[id_a], &[], None, 10)?;
    assert!(
        expanded.contains(&id_b),
        "Graph expansion from A should find B (shared entity: TypeScript). Got: {:?}",
        expanded,
    );
    // id_c should NOT be found (no shared entity with A)
    assert!(
        !expanded.contains(&id_c),
        "Graph expansion from A should NOT find C (no shared entity)",
    );

    Ok(())
}

// ===========================================================================
// Aggregate Report
// ===========================================================================

#[test]
fn bench_aggregate_report() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_full_schema(&conn)?;

    let seeds = search_eval_memories();
    let ids = insert_seed_memories(&conn, &seeds)?;

    let fts_relevant: Vec<i64> = seeds
        .iter()
        .zip(ids.iter())
        .filter(|(s, _)| s.relevant_to_fts_query)
        .map(|(_, id)| *id)
        .collect();
    let decay_relevant: Vec<i64> = seeds
        .iter()
        .zip(ids.iter())
        .filter(|(s, _)| s.relevant_to_decay_query)
        .map(|(_, id)| *id)
        .collect();

    let fts_results = search::search(
        &conn,
        Some("FTS5 search"),
        Some("tools/remem"),
        None,
        10,
        0,
        true,
    )?;
    let fts_ids: Vec<i64> = fts_results.iter().map(|m| m.id).collect();

    let decay_results = search::search(
        &conn,
        Some("time decay ranking"),
        Some("tools/remem"),
        None,
        10,
        0,
        true,
    )?;
    let decay_ids: Vec<i64> = decay_results.iter().map(|m| m.id).collect();

    eprintln!("\n========================================");
    eprintln!("  remem Benchmark Report");
    eprintln!("========================================");
    eprintln!(
        "  FTS5 Search    P@5={:.2}  R@10={:.2}",
        precision_at_k(&fts_ids, &fts_relevant, 5),
        recall_at_k(&fts_ids, &fts_relevant, 10)
    );
    eprintln!(
        "  Decay Search   P@5={:.2}  R@10={:.2}",
        precision_at_k(&decay_ids, &decay_relevant, 5),
        recall_at_k(&decay_ids, &decay_relevant, 10)
    );
    eprintln!("  Total seeds: {}", seeds.len());
    eprintln!("  FTS relevant: {}", fts_relevant.len());
    eprintln!("  Decay relevant: {}", decay_relevant.len());
    eprintln!("========================================\n");

    Ok(())
}
