//! v074 git commit staleness lookup migration tests (GH-948).

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::db::test_support::ScopedTestDataDir;

use super::run::{run_post_migration_hook, run_pre_migration_hook};
use super::state::{applied_versions, ensure_migration_table, mark_applied};
use super::{run_migrations, MIGRATIONS};

const V074: i64 = 74;

fn pre_v074(label: &str) -> Result<(ScopedTestDataDir, Connection)> {
    let data_dir = ScopedTestDataDir::new(label);
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    ensure_migration_table(&conn)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| {
        for migration in MIGRATIONS
            .iter()
            .filter(|migration| migration.version < V074)
        {
            run_pre_migration_hook(&conn, migration.version, migration.name)?;
            conn.execute_batch(migration.sql).with_context(|| {
                format!(
                    "apply pre-v074 migration v{:03}_{}",
                    migration.version, migration.name
                )
            })?;
            run_post_migration_hook(&conn, migration.version, migration.name)?;
            mark_applied(&conn, migration.version, migration.name)?;
        }
        Ok::<_, anyhow::Error>(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT")?,
        Err(error) => {
            conn.execute_batch("ROLLBACK")?;
            return Err(error);
        }
    }
    Ok((data_dir, conn))
}

fn insert_git_commit_fixture(
    conn: &Connection,
    sha: &str,
    authored_at_epoch: Option<i64>,
    changed_files: &str,
    created_at_epoch: i64,
    updated_at_epoch: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO git_commits(
             project, repo_path, sha, short_sha, branch, changed_files,
             authored_at_epoch, created_at_epoch, updated_at_epoch
         ) VALUES (
             '/repo', '/repo', ?1, substr(?1, 1, 7), 'main', ?2, ?3, ?4, ?5
         )",
        params![
            sha,
            changed_files,
            authored_at_epoch,
            created_at_epoch,
            updated_at_epoch
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn commit_files(conn: &Connection, commit_id: i64) -> Result<Vec<String>> {
    let mut statement =
        conn.prepare("SELECT path FROM git_commit_files WHERE commit_id = ?1 ORDER BY path")?;
    let files = statement
        .query_map([commit_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(files)
}

#[test]
fn v074_remains_registered_after_later_schema_versions() {
    assert!(
        super::latest_schema_version() >= V074,
        "v074 must not be dropped by a later schema version"
    );
    assert_eq!(
        MIGRATIONS
            .iter()
            .find(|migration| migration.version == V074)
            .map(|migration| migration.name),
        Some("git_commit_staleness_index")
    );
}

#[test]
fn migration_backfills_commit_files_and_adds_epoch_index() -> Result<()> {
    let (_data_dir, conn) = pre_v074("staleness-index-backfill")?;
    let authored = insert_git_commit_fixture(
        &conn,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        Some(300),
        r#"["src/a.rs","src/b.rs","src/a.rs"," ./legacy.rs ",""]"#,
        100,
        200,
    )?;
    let updated = insert_git_commit_fixture(
        &conn,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        None,
        r#"["README.md"]"#,
        150,
        250,
    )?;
    let empty = insert_git_commit_fixture(
        &conn,
        "cccccccccccccccccccccccccccccccccccccccc",
        None,
        "[]",
        175,
        175,
    )?;

    run_migrations(&conn)?;

    assert_eq!(
        commit_files(&conn, authored)?,
        vec![
            "".to_string(),
            " ./legacy.rs ".to_string(),
            "src/a.rs".to_string(),
            "src/b.rs".to_string(),
        ]
    );
    assert_eq!(commit_files(&conn, updated)?, vec!["README.md"]);
    assert!(commit_files(&conn, empty)?.is_empty());

    let ordered: Vec<String> = conn
        .prepare(
            "SELECT sha
             FROM git_commits
             WHERE project = '/repo'
               AND COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) > 200
             ORDER BY COALESCE(authored_at_epoch, updated_at_epoch, created_at_epoch) DESC,
                      id DESC",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(
        ordered,
        vec![
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        ]
    );

    let indexes: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master
             WHERE type = 'index'
               AND name = 'idx_git_commits_project_commit_epoch'
             ORDER BY name",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(indexes, vec!["idx_git_commits_project_commit_epoch"]);
    Ok(())
}

#[test]
fn malformed_history_rolls_back_the_entire_migration() -> Result<()> {
    for (label, payload) in [
        ("malformed", r#"["src/lib.rs""#),
        ("object", r#"{"path":"src/lib.rs"}"#),
        ("non-string", r#"["src/lib.rs",7]"#),
    ] {
        let (_data_dir, conn) = pre_v074(&format!("staleness-index-{label}"))?;
        let sha = format!("{label:-<40}");
        let bad_id = insert_git_commit_fixture(&conn, &sha, Some(300), payload, 100, 200)?;

        let error = run_migrations(&conn).expect_err("invalid changed_files must fail migration");
        let message = format!("{error:#}");
        assert!(
            message.contains(&format!("git_commits.id={bad_id}")) && message.contains(&sha),
            "migration error must identify the invalid {label} commit: {message}"
        );
        assert!(
            !applied_versions(&conn)?.contains(&V074),
            "failed migration must not be marked applied"
        );
        let table_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'git_commit_files'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(table_exists, 0, "failed migration must roll back its DDL");
    }
    Ok(())
}

#[test]
fn legacy_column_only_writes_keep_commit_files_synchronized() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    run_migrations(&conn)?;

    let commit_id = insert_git_commit_fixture(
        &conn,
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        Some(300),
        r#"["src/old.rs","README.md"]"#,
        100,
        200,
    )?;
    assert_eq!(
        commit_files(&conn, commit_id)?,
        vec!["README.md".to_string(), "src/old.rs".to_string()]
    );

    conn.execute(
        "INSERT INTO git_commits(
             project, repo_path, sha, short_sha, branch, changed_files,
             authored_at_epoch, created_at_epoch, updated_at_epoch
         ) VALUES (
             '/repo', '/repo', ?1, substr(?1, 1, 7), 'main',
             '[\"src/new.rs\"]', 400, 100, 400
         )
         ON CONFLICT(project, sha) DO UPDATE SET
             changed_files = excluded.changed_files,
             updated_at_epoch = excluded.updated_at_epoch",
        ["eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"],
    )?;
    assert_eq!(commit_files(&conn, commit_id)?, vec!["src/new.rs"]);

    conn.execute("DELETE FROM git_commits WHERE id = ?1", [commit_id])?;
    assert!(commit_files(&conn, commit_id)?.is_empty());
    Ok(())
}

#[test]
fn changed_files_triggers_reject_invalid_new_payloads() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;

    for (suffix, payload) in [
        ("object", r#"{"path":"src/lib.rs"}"#),
        ("number", r#"["src/lib.rs", 7]"#),
        ("malformed", r#"["src/lib.rs""#),
    ] {
        let sha = format!("{suffix:0<40}");
        let error = insert_git_commit_fixture(&conn, &sha, Some(300), payload, 100, 200)
            .expect_err("invalid changed_files must be rejected");
        assert!(
            error
                .to_string()
                .contains("git_commits.changed_files must be a JSON array of strings"),
            "unexpected trigger error for {suffix}: {error:#}"
        );
    }

    let valid_id = insert_git_commit_fixture(
        &conn,
        "valid-update-payload--------------------",
        Some(300),
        r#"["src/original.rs"]"#,
        100,
        200,
    )?;
    for (suffix, payload) in [
        ("object", r#"{"path":"src/lib.rs"}"#),
        ("number", r#"["src/lib.rs", 7]"#),
        ("malformed", r#"["src/lib.rs""#),
    ] {
        let error = conn
            .execute(
                "UPDATE git_commits SET changed_files = ?1 WHERE id = ?2",
                params![payload, valid_id],
            )
            .expect_err("invalid changed_files update must be rejected");
        assert!(
            error
                .to_string()
                .contains("git_commits.changed_files must be a JSON array of strings"),
            "unexpected update trigger error for {suffix}: {error:#}"
        );
        assert_eq!(
            commit_files(&conn, valid_id)?,
            vec!["src/original.rs"],
            "rejected update must preserve derived paths"
        );
    }
    Ok(())
}

#[test]
fn changed_files_lookup_preserves_legacy_path_spellings() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    let commit_id = insert_git_commit_fixture(
        &conn,
        "ffffffffffffffffffffffffffffffffffffffff",
        Some(300),
        r#"["","\t"," ./src/lib.rs ","/repo/src/main.rs"]"#,
        100,
        200,
    )?;

    assert_eq!(
        commit_files(&conn, commit_id)?,
        vec![
            "".to_string(),
            "\t".to_string(),
            " ./src/lib.rs ".to_string(),
            "/repo/src/main.rs".to_string(),
        ]
    );
    Ok(())
}
