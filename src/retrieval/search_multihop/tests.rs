use crate::memory::Memory;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::discover::discover_entities;
use super::merge::rank_merged_ids;

fn make_memory(id: i64, title: &str, text: &str) -> Memory {
    Memory {
        id,
        session_id: None,
        project: "proj".to_string(),
        topic_key: None,
        title: title.to_string(),
        text: text.to_string(),
        memory_type: "discovery".to_string(),
        files: None,
        created_at_epoch: 0,
        updated_at_epoch: 0,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}

#[test]
fn discover_entities_skips_query_entities_and_deduplicates() {
    let first_hop = vec![
        make_memory(1, "Melanie", "Tom Sarah"),
        make_memory(2, "Tom", "Sarah Tom"),
    ];

    let entities = discover_entities("Melanie", &first_hop);
    assert_eq!(entities, vec!["Tom", "Sarah"]);
}

#[test]
fn rank_merged_ids_boosts_overlap_and_respects_limit() {
    let ranked = rank_merged_ids(&[1, 2], &[2, 3], 3);
    assert_eq!(ranked, vec![2, 1, 3]);

    let limited = rank_merged_ids(&[1, 2], &[2, 3], 2);
    assert_eq!(limited, vec![2, 1]);
}

#[test]
fn search_multi_hop_demotes_verify_before_trust_second_hop() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let first_id = crate::memory::insert_memory_full(
        &conn,
        Some("first-session"),
        "proj",
        Some("multi-hop-first"),
        "Melanie route",
        "Melanie mentions Tom in routing notes.",
        "discovery",
        None,
        Some("main"),
        "project",
        Some(150),
    )?;
    let stale_id = crate::memory::insert_memory_full(
        &conn,
        Some("stale-session"),
        "proj",
        Some("multi-hop-stale"),
        "Tom stale source",
        "Tom stale source-anchor memory.",
        "decision",
        Some(r#"["src/stale.rs"]"#),
        Some("main"),
        "project",
        Some(300),
    )?;
    let fresh_id = crate::memory::insert_memory_full(
        &conn,
        Some("fresh-session"),
        "proj",
        Some("multi-hop-fresh"),
        "Tom fresh source",
        "Tom fresh source-anchor memory.",
        "decision",
        Some(r#"["src/fresh.rs"]"#),
        Some("main"),
        "project",
        Some(150),
    )?;
    crate::retrieval::entity::link_entities(&conn, stale_id, &["Tom".to_string()])?;
    crate::retrieval::entity::link_entities(&conn, fresh_id, &["Tom".to_string()])?;
    link_commit(
        &conn,
        1,
        "source-stale",
        100,
        &["src/stale.rs"],
        "stale-session",
    )?;
    insert_commit(&conn, 2, "later-stale", 200, &["src/stale.rs"])?;
    link_commit(
        &conn,
        3,
        "source-fresh",
        100,
        &["src/fresh.rs"],
        "fresh-session",
    )?;

    let result = super::search::search_multi_hop(
        &conn,
        "Melanie",
        Some("proj"),
        3,
        0,
        None,
        Some("main"),
        false,
    )?;
    let ids = result
        .memories
        .iter()
        .map(|memory| memory.id)
        .collect::<Vec<_>>();

    assert_eq!(result.hops, 2);
    assert_eq!(ids.first().copied(), Some(first_id));
    let fresh_rank = ids
        .iter()
        .position(|id| *id == fresh_id)
        .context("fresh second-hop memory missing")?;
    let stale_rank = ids
        .iter()
        .position(|id| *id == stale_id)
        .context("stale second-hop memory missing")?;
    assert!(fresh_rank < stale_rank, "{ids:?}");
    Ok(())
}

fn link_commit(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    memory_session_id: &str,
) -> Result<()> {
    insert_commit(conn, id, sha, epoch, changed_files)?;
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, ?2, ?3, 'test', ?4)",
        params![id, format!("content-{id}"), memory_session_id, epoch],
    )?;
    Ok(())
}

fn insert_commit(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
) -> Result<()> {
    let changed_files = serde_json::to_string(changed_files)?;
    conn.execute(
        "INSERT INTO git_commits
         (id, project, repo_path, sha, short_sha, branch, message,
          authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'proj', '/repo', ?2, ?2, 'main', NULL, ?3, ?4, ?3, ?3)",
        params![id, sha, epoch, changed_files],
    )?;
    Ok(())
}
