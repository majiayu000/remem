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
    for expected in ["fts", "entity", "temporal", "like_fallback"] {
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
        !result.contributions.is_empty()
            && result
                .contributions
                .iter()
                .all(|contribution| contribution.score > 0.0)
    }));
    Ok(())
}
