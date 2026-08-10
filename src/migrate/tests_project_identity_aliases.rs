use anyhow::Result;
use rusqlite::{params, Connection};

use super::{run_migrations, validate_schema_invariants};

#[test]
fn v081_records_append_only_alias_proof_and_current_registry() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    run_migrations(&conn)?;
    assert!(validate_schema_invariants(&conn)?.is_empty());

    conn.execute(
        "INSERT INTO workspaces(
            root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch
         ) VALUES('/new/repo', 'https://github.com/o/r.git', 'main', 1, 1)",
        [],
    )?;
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(
            workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch
         ) VALUES(?1, '/new/repo', '/new/repo', 1, 1)",
        [workspace_id],
    )?;
    let project_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO project_identity_alias_events(
            alias_path, canonical_project_id, action, proof_kind,
            proof_payload_json, proof_sha256, source_inventory_sha256,
            actor, reason, created_at_epoch
         ) VALUES(?1, ?2, 'activate', 'git_commit_membership', '{}', ?3, ?4,
                  'test', 'fixture', 1)",
        params!["/old/repo", project_id, "a".repeat(64), "b".repeat(64)],
    )?;
    let event_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO project_identity_aliases(
            alias_path, canonical_project_id, status, last_event_id,
            created_at_epoch, updated_at_epoch
         ) VALUES('/old/repo', ?1, 'active', ?2, 1, 1)",
        params![project_id, event_id],
    )?;

    let resolved: (String, String, String) = conn.query_row(
        "SELECT aliases.alias_path, projects.project_path, events.proof_kind
         FROM project_identity_aliases aliases
         JOIN projects ON projects.id = aliases.canonical_project_id
         JOIN project_identity_alias_events events ON events.id = aliases.last_event_id
         WHERE aliases.alias_path = '/old/repo' AND aliases.status = 'active'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(
        resolved,
        (
            "/old/repo".to_string(),
            "/new/repo".to_string(),
            "git_commit_membership".to_string()
        )
    );
    Ok(())
}

#[test]
fn v081_rejects_unapproved_proof_kind_and_malformed_digest() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    run_migrations(&conn)?;
    conn.execute(
        "INSERT INTO workspaces(
            root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch
         ) VALUES('/new/repo', NULL, NULL, 1, 1)",
        [],
    )?;
    let workspace_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO projects(
            workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch
         ) VALUES(?1, '/new/repo', '/new/repo', 1, 1)",
        [workspace_id],
    )?;
    let project_id = conn.last_insert_rowid();

    let bad_kind = conn.execute(
        "INSERT INTO project_identity_alias_events(
            alias_path, canonical_project_id, action, proof_kind,
            proof_payload_json, proof_sha256, source_inventory_sha256,
            actor, reason, created_at_epoch
         ) VALUES('/old/repo', ?1, 'activate', 'model_guess', '{}', ?2, ?3,
                  'test', 'fixture', 1)",
        params![project_id, "a".repeat(64), "b".repeat(64)],
    );
    assert!(bad_kind.is_err());

    let bad_digest = conn.execute(
        "INSERT INTO project_identity_alias_events(
            alias_path, canonical_project_id, action, proof_kind,
            proof_payload_json, proof_sha256, source_inventory_sha256,
            actor, reason, created_at_epoch
         ) VALUES('/old/repo', ?1, 'activate', 'git_remote', '{}', 'xyz', ?2,
                  'test', 'fixture', 1)",
        params![project_id, "b".repeat(64)],
    );
    assert!(bad_digest.is_err());
    Ok(())
}
