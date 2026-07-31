use anyhow::Result;
use rusqlite::{params, Connection, StatementStatus};

use super::*;

const PROJECT: &str = "/repo";
const ANCHOR_EPOCH: i64 = 20_000;
const ANCHOR_ID: i64 = 20_000;

#[test]
fn later_commit_plan_uses_commit_epoch_range_index() -> Result<()> {
    let conn = migrated_staleness_query_connection()?;
    let explain_sql = format!("EXPLAIN QUERY PLAN {LATER_COMMIT_FILES_SQL}");
    let mut statement = conn.prepare(&explain_sql)?;
    let details = statement
        .query_map(
            params![PROJECT, ANCHOR_EPOCH, ANCHOR_ID, Option::<&str>::None],
            |row| row.get::<_, String>(3),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .join("\n");

    assert!(
        details.contains("idx_git_commits_project_commit_epoch"),
        "later-commit plan must use the epoch index:\n{details}"
    );
    assert!(
        details.contains("<expr>>?"),
        "later-commit plan must constrain the indexed epoch range:\n{details}"
    );
    assert!(
        details.contains("<expr>=? AND id>?"),
        "later-commit plan must seek past the id boundary for equal epochs:\n{details}"
    );
    Ok(())
}

#[test]
fn source_commit_plan_starts_from_session_links() -> Result<()> {
    let conn = migrated_staleness_query_connection()?;
    let explain_sql = format!("EXPLAIN QUERY PLAN {SOURCE_COMMIT_FILES_SQL}");
    let mut statement = conn.prepare(&explain_sql)?;
    let details = statement
        .query_map(
            params![
                PROJECT,
                "memory-session",
                Option::<&str>::None,
                ANCHOR_EPOCH
            ],
            |row| row.get::<_, String>(3),
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let commit_search = details
        .iter()
        .position(|detail| detail.contains("SEARCH c USING INTEGER PRIMARY KEY"))
        .expect("source plan must resolve linked commits by primary key");
    let link_search = details
        .iter()
        .position(|detail| detail.contains("git_commit_sessions"))
        .expect("source plan must search session links");

    assert!(
        link_search < commit_search,
        "source plan must search session links before commit rows:\n{}",
        details.join("\n")
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("idx_git_commit_sessions_memory_session")),
        "source plan must use the memory-session index:\n{}",
        details.join("\n")
    );
    assert!(
        details
            .iter()
            .any(|detail| detail.contains("idx_git_commit_sessions_session")),
        "source plan must use the legacy session index:\n{}",
        details.join("\n")
    );
    Ok(())
}

#[test]
fn later_commit_work_is_independent_of_pre_anchor_history() -> Result<()> {
    let small = seeded_history(0)?;
    let large = seeded_history(10_000)?;
    let (small_steps, mut small_paths) = later_query_vm_steps(&small)?;
    let (large_steps, mut large_paths) = later_query_vm_steps(&large)?;
    small_paths.sort();
    large_paths.sort();

    assert_eq!(small_paths, vec!["src/one.rs", "src/two.rs"]);
    assert_eq!(large_paths, small_paths);
    assert!(
        large_steps <= small_steps + 16,
        "pre-anchor history changed later-commit work: small={small_steps} large={large_steps}"
    );
    Ok(())
}

#[test]
fn source_commit_work_is_independent_of_unlinked_project_history() -> Result<()> {
    let small = seeded_source_history(0)?;
    let large = seeded_source_history(10_000)?;
    for (sql, expected_path) in [
        (SOURCE_COMMIT_FILES_SQL, "src/anchor.rs"),
        (SOURCE_COMMIT_JSON_SQL, r#"["src/anchor.rs"]"#),
    ] {
        let (small_steps, small_rows) = source_query_vm_steps(&small, sql)?;
        let (large_steps, large_rows) = source_query_vm_steps(&large, sql)?;

        assert_eq!(small_rows, vec![(ANCHOR_ID, expected_path.to_string())]);
        assert_eq!(large_rows, small_rows);
        assert!(
            large_steps <= small_steps + 32,
            "unlinked project history changed source-anchor work: \
             small={small_steps} large={large_steps}"
        );
    }
    Ok(())
}

#[test]
fn later_commit_work_is_independent_of_same_epoch_pre_anchor_history() -> Result<()> {
    let small = seeded_same_epoch_history(0)?;
    let large = seeded_same_epoch_history(10_000)?;
    let (small_steps, mut small_paths) = later_query_vm_steps(&small)?;
    let (large_steps, mut large_paths) = later_query_vm_steps(&large)?;
    small_paths.sort();
    large_paths.sort();

    assert_eq!(small_paths, vec!["src/one.rs", "src/two.rs"]);
    assert_eq!(large_paths, small_paths);
    assert!(
        large_steps <= small_steps + 32,
        "same-epoch pre-anchor history changed later-commit work: \
         small={small_steps} large={large_steps}"
    );
    Ok(())
}

#[test]
fn optimized_queries_preserve_path_normalization_and_epoch_tie_break() -> Result<()> {
    let conn = migrated_staleness_query_connection()?;
    insert_query_commit_fixture(&conn, 10, 100, Some("main"), r#"[" ./src/lib.rs "]"#)?;
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (10, 'content-10', 'memory-session', 'test', 100)",
        [],
    )?;
    insert_query_commit_fixture(&conn, 11, 100, Some("main"), r#"["/repo/src/lib.rs"]"#)?;

    let anchor = source_commit_anchor_for_session(
        &conn,
        PROJECT,
        "memory-session",
        Some("main"),
        100,
        "src/lib.rs",
        true,
    )?
    .expect("source path variant must remain anchored");
    assert_eq!((anchor.epoch, anchor.id), (100, 10));
    assert!(
        later_commit_touches_file(&conn, PROJECT, &anchor, Some("main"), "src/lib.rs", true,)?,
        "same-epoch commit with a larger id and equivalent absolute path must be later"
    );
    Ok(())
}

fn migrated_staleness_query_connection() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

fn seeded_history(pre_anchor_count: i64) -> Result<Connection> {
    let mut conn = migrated_staleness_query_connection()?;
    let transaction = conn.transaction()?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO git_commits(
                 id, project, repo_path, sha, short_sha, branch, changed_files,
                 authored_at_epoch, created_at_epoch, updated_at_epoch
             ) VALUES (?1, ?2, ?2, ?3, ?3, 'main', ?4, ?5, ?5, ?5)",
        )?;
        for id in 1..=pre_anchor_count {
            statement.execute(params![id, PROJECT, format!("pre-{id}"), "[]", id])?;
        }
        for (id, epoch, path) in [
            (ANCHOR_ID + 1, ANCHOR_EPOCH + 1, "src/one.rs"),
            (ANCHOR_ID + 2, ANCHOR_EPOCH + 2, "src/two.rs"),
        ] {
            statement.execute(params![
                id,
                PROJECT,
                format!("post-{id}"),
                serde_json::to_string(&[path])?,
                epoch
            ])?;
        }
    }
    transaction.commit()?;
    Ok(conn)
}

fn seeded_source_history(unlinked_count: i64) -> Result<Connection> {
    let mut conn = migrated_staleness_query_connection()?;
    let transaction = conn.transaction()?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO git_commits(
                 id, project, repo_path, sha, short_sha, branch, changed_files,
                 authored_at_epoch, created_at_epoch, updated_at_epoch
             ) VALUES (?1, ?2, ?2, ?3, ?3, 'main', ?4, ?5, ?5, ?5)",
        )?;
        insert_query_commit_with_statement(&mut statement, ANCHOR_ID, 1, "src/anchor.rs")?;
        for id in 1..=unlinked_count {
            insert_query_commit_with_statement(&mut statement, id, id + 1, "src/unlinked.rs")?;
        }
        transaction.execute(
            "INSERT INTO git_commit_sessions(
                 commit_id, session_id, memory_session_id, source, linked_at_epoch
             ) VALUES (?1, 'content-anchor', 'memory-session', 'test', 1)",
            [ANCHOR_ID],
        )?;
    }
    transaction.commit()?;
    Ok(conn)
}

fn seeded_same_epoch_history(pre_anchor_count: i64) -> Result<Connection> {
    let mut conn = migrated_staleness_query_connection()?;
    let transaction = conn.transaction()?;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO git_commits(
                 id, project, repo_path, sha, short_sha, branch, changed_files,
                 authored_at_epoch, created_at_epoch, updated_at_epoch
             ) VALUES (?1, ?2, ?2, ?3, ?3, 'main', ?4, ?5, ?5, ?5)",
        )?;
        for id in 1..=pre_anchor_count {
            insert_query_commit_with_statement(&mut statement, id, ANCHOR_EPOCH, "src/pre.rs")?;
        }
        for (id, path) in [(ANCHOR_ID + 1, "src/one.rs"), (ANCHOR_ID + 2, "src/two.rs")] {
            insert_query_commit_with_statement(&mut statement, id, ANCHOR_EPOCH, path)?;
        }
    }
    transaction.commit()?;
    Ok(conn)
}

fn later_query_vm_steps(conn: &Connection) -> Result<(i32, Vec<String>)> {
    let mut statement = conn.prepare(LATER_COMMIT_FILES_SQL)?;
    statement.reset_status(StatementStatus::VmStep);
    let paths = {
        let rows = statement.query_map(
            params![PROJECT, ANCHOR_EPOCH, ANCHOR_ID, Option::<&str>::None],
            |row| row.get::<_, String>(0),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok((statement.get_status(StatementStatus::VmStep), paths))
}

fn source_query_vm_steps(conn: &Connection, sql: &str) -> Result<(i32, Vec<(i64, String)>)> {
    let mut statement = conn.prepare(sql)?;
    statement.reset_status(StatementStatus::VmStep);
    let rows = {
        let rows = statement.query_map(
            params![
                PROJECT,
                "memory-session",
                Option::<&str>::None,
                ANCHOR_EPOCH
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(3)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    Ok((statement.get_status(StatementStatus::VmStep), rows))
}

fn insert_query_commit_with_statement(
    statement: &mut rusqlite::Statement<'_>,
    id: i64,
    epoch: i64,
    path: &str,
) -> Result<()> {
    statement.execute(params![
        id,
        PROJECT,
        format!("sha-{id}"),
        serde_json::to_string(&[path])?,
        epoch
    ])?;
    Ok(())
}

fn insert_query_commit_fixture(
    conn: &Connection,
    id: i64,
    epoch: i64,
    branch: Option<&str>,
    changed_files: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO git_commits(
             id, project, repo_path, sha, short_sha, branch, changed_files,
             authored_at_epoch, created_at_epoch, updated_at_epoch
         ) VALUES (?1, ?2, ?2, ?3, ?3, ?4, ?5, ?6, ?6, ?6)",
        params![
            id,
            PROJECT,
            format!("sha-{id}"),
            branch,
            changed_files,
            epoch
        ],
    )?;
    Ok(())
}
