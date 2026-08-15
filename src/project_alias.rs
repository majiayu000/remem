//! Audited project path aliases over the canonical `projects` capture identity.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectAliasProofKind {
    FilesystemCanonicalization,
    GitRemote,
    GitCommitMembership,
}

impl ProjectAliasProofKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::FilesystemCanonicalization => "filesystem_canonicalization",
            Self::GitRemote => "git_remote",
            Self::GitCommitMembership => "git_commit_membership",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectAliasPlanEntry {
    pub alias_path: String,
    pub canonical_path: String,
    pub proof_kind: ProjectAliasProofKind,
    pub proof_payload: Value,
    pub proof_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ProjectAliasApplyRequest<'a> {
    pub source_inventory_sha256: &'a str,
    pub actor: &'a str,
    pub reason: &'a str,
    pub now_epoch: i64,
    pub entries: &'a [ProjectAliasPlanEntry],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectAliasApplyResult {
    pub inserted: usize,
    pub unchanged: usize,
    pub aliases: Vec<ProjectAliasResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectAliasResolution {
    pub requested_path: String,
    pub canonical_project_id: Option<i64>,
    pub canonical_path: String,
    pub active_aliases: Vec<String>,
    pub resolved_via_alias: bool,
}

pub fn proof_sha256(payload: &Value) -> Result<String> {
    let encoded = serde_json::to_vec(payload).context("serialize project alias proof payload")?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

pub fn apply_project_alias_plan(
    conn: &Connection,
    request: &ProjectAliasApplyRequest<'_>,
) -> Result<ProjectAliasApplyResult> {
    validate_request_header(request)?;

    let tx = conn.unchecked_transaction()?;
    let mut inserted = 0;
    let mut unchanged = 0;
    let mut resolved = Vec::new();
    for entry in request.entries {
        validate_entry(&tx, entry)?;
        let canonical_project_id =
            exact_project_id(&tx, &entry.canonical_path)?.ok_or_else(|| {
                anyhow::anyhow!("canonical project not found: {}", entry.canonical_path)
            })?;
        if active_alias_target(&tx, &entry.canonical_path)?.is_some() {
            bail!(
                "project alias chains are forbidden: target {} is itself an active alias",
                entry.canonical_path
            );
        }

        match active_alias_target(&tx, &entry.alias_path)? {
            Some(existing) if existing == canonical_project_id => {
                unchanged += 1;
            }
            Some(existing) => bail!(
                "project alias collision: {} already targets project id {}, requested {}",
                entry.alias_path,
                existing,
                canonical_project_id
            ),
            None => {
                let payload_json = serde_json::to_string(&entry.proof_payload)?;
                tx.execute(
                    "INSERT INTO project_identity_alias_events(
                        alias_path, canonical_project_id, action, proof_kind,
                        proof_payload_json, proof_sha256, source_inventory_sha256,
                        actor, reason, created_at_epoch
                     ) VALUES(?1, ?2, 'activate', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        entry.alias_path,
                        canonical_project_id,
                        entry.proof_kind.as_str(),
                        payload_json,
                        entry.proof_sha256,
                        request.source_inventory_sha256,
                        request.actor,
                        request.reason,
                        request.now_epoch,
                    ],
                )?;
                let event_id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO project_identity_aliases(
                        alias_path, canonical_project_id, status, last_event_id,
                        created_at_epoch, updated_at_epoch
                     ) VALUES(?1, ?2, 'active', ?3, ?4, ?4)
                     ON CONFLICT(alias_path) DO UPDATE SET
                        canonical_project_id = excluded.canonical_project_id,
                        status = 'active',
                        last_event_id = excluded.last_event_id,
                        updated_at_epoch = excluded.updated_at_epoch",
                    params![
                        entry.alias_path,
                        canonical_project_id,
                        event_id,
                        request.now_epoch
                    ],
                )?;
                inserted += 1;
            }
        }
        resolved.push(resolve_project_identity(&tx, &entry.alias_path)?);
    }
    tx.commit()?;
    resolved.sort_by(|a, b| a.requested_path.cmp(&b.requested_path));
    Ok(ProjectAliasApplyResult {
        inserted,
        unchanged,
        aliases: resolved,
    })
}

pub fn preview_project_alias_plan(
    conn: &Connection,
    request: &ProjectAliasApplyRequest<'_>,
) -> Result<ProjectAliasApplyResult> {
    validate_request_header(request)?;
    let mut inserted = 0;
    let mut unchanged = 0;
    let mut resolved = Vec::new();
    for entry in request.entries {
        validate_entry(conn, entry)?;
        let canonical_project_id =
            exact_project_id(conn, &entry.canonical_path)?.ok_or_else(|| {
                anyhow::anyhow!("canonical project not found: {}", entry.canonical_path)
            })?;
        if active_alias_target(conn, &entry.canonical_path)?.is_some() {
            bail!(
                "project alias chains are forbidden: target {} is itself an active alias",
                entry.canonical_path
            );
        }
        match active_alias_target(conn, &entry.alias_path)? {
            Some(existing) if existing == canonical_project_id => {
                unchanged += 1;
                resolved.push(resolve_project_identity(conn, &entry.alias_path)?);
            }
            Some(existing) => bail!(
                "project alias collision: {} already targets project id {}, requested {}",
                entry.alias_path,
                existing,
                canonical_project_id
            ),
            None => {
                inserted += 1;
                let mut active_aliases = active_aliases_for_project(conn, canonical_project_id)?;
                active_aliases.push(entry.alias_path.clone());
                active_aliases.sort();
                active_aliases.dedup();
                resolved.push(ProjectAliasResolution {
                    requested_path: entry.alias_path.clone(),
                    canonical_project_id: Some(canonical_project_id),
                    canonical_path: entry.canonical_path.clone(),
                    active_aliases,
                    resolved_via_alias: true,
                });
            }
        }
    }
    resolved.sort_by(|a, b| a.requested_path.cmp(&b.requested_path));
    Ok(ProjectAliasApplyResult {
        inserted,
        unchanged,
        aliases: resolved,
    })
}

fn validate_request_header(request: &ProjectAliasApplyRequest<'_>) -> Result<()> {
    validate_digest(request.source_inventory_sha256, "source_inventory_sha256")?;
    if request.actor.trim().is_empty() {
        bail!("project alias apply requires a non-empty actor");
    }
    if request.reason.trim().is_empty() {
        bail!("project alias apply requires a non-empty reason");
    }
    if request.entries.is_empty() {
        bail!("project alias apply requires at least one entry");
    }
    Ok(())
}

pub fn resolve_project_identity(
    conn: &Connection,
    requested_path: &str,
) -> Result<ProjectAliasResolution> {
    if !alias_registry_available(conn)? {
        return Ok(ProjectAliasResolution {
            requested_path: requested_path.to_string(),
            canonical_project_id: None,
            canonical_path: requested_path.to_string(),
            active_aliases: Vec::new(),
            resolved_via_alias: false,
        });
    }
    let alias_target = conn
        .query_row(
            "SELECT aliases.canonical_project_id, projects.project_path
             FROM project_identity_aliases aliases
             JOIN projects ON projects.id = aliases.canonical_project_id
             WHERE aliases.alias_path = ?1 AND aliases.status = 'active'",
            [requested_path],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let (canonical_project_id, canonical_path, resolved_via_alias) = match alias_target {
        Some((id, path)) => (Some(id), path, true),
        None => match exact_project_id(conn, requested_path)? {
            Some(id) => (Some(id), requested_path.to_string(), false),
            None => (None, requested_path.to_string(), false),
        },
    };
    let active_aliases = match canonical_project_id {
        Some(id) => active_aliases_for_project(conn, id)?,
        None => Vec::new(),
    };
    Ok(ProjectAliasResolution {
        requested_path: requested_path.to_string(),
        canonical_project_id,
        canonical_path,
        active_aliases,
        resolved_via_alias,
    })
}

fn active_aliases_for_project(conn: &Connection, id: i64) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT alias_path FROM project_identity_aliases
         WHERE canonical_project_id = ?1 AND status = 'active'
         ORDER BY alias_path",
    )?;
    let aliases = statement
        .query_map([id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(aliases)
}

pub fn project_filter_values(conn: &Connection, requested_path: &str) -> Result<Vec<String>> {
    let resolution = resolve_project_identity(conn, requested_path)?;
    let mut values = resolution.active_aliases;
    values.push(resolution.canonical_path);
    values.sort();
    values.dedup();
    Ok(values)
}

/// Build a bound `IN` predicate containing the canonical project path and all
/// active historical aliases. The registry lookup falls back to the requested
/// value when called against a deliberately partial test schema.
pub fn push_project_value_filter(
    conn: &Connection,
    column: &str,
    requested_path: &str,
    mut idx: usize,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> Result<(String, usize)> {
    let values = project_filter_values(conn, requested_path)?;
    let values_json = serde_json::to_string(&values)?;
    params.push(Box::new(values_json));
    let predicate = format!("{column} IN (SELECT value FROM json_each(?{idx}))");
    idx += 1;
    Ok((predicate, idx))
}

/// Resolve a project-bearing value before a new row is written. Historical
/// rows remain untouched; only subsequent writes converge on the canonical
/// project registered in `projects`.
pub fn canonical_project_path_for_write(conn: &Connection, requested_path: &str) -> Result<String> {
    Ok(resolve_project_identity(conn, requested_path)?.canonical_path)
}

pub(crate) fn alias_registry_available(conn: &Connection) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type = 'table'
           AND name IN ('projects', 'project_identity_aliases')",
        [],
        |row| row.get(0),
    )?;
    Ok(count == 2)
}

fn validate_entry(conn: &Connection, entry: &ProjectAliasPlanEntry) -> Result<()> {
    if !Path::new(&entry.alias_path).is_absolute()
        || !Path::new(&entry.canonical_path).is_absolute()
    {
        bail!("project alias paths must be absolute");
    }
    if entry.alias_path == entry.canonical_path {
        bail!("project alias source and canonical path must differ");
    }
    validate_digest(&entry.proof_sha256, "proof_sha256")?;
    let actual = proof_sha256(&entry.proof_payload)?;
    if actual != entry.proof_sha256 {
        bail!("project alias proof digest does not match payload");
    }
    let payload = entry
        .proof_payload
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("project alias proof payload must be an object"))?;
    if payload.get("from_path").and_then(Value::as_str) != Some(entry.alias_path.as_str())
        || payload.get("to_path").and_then(Value::as_str) != Some(entry.canonical_path.as_str())
    {
        bail!("project alias proof payload path binding does not match plan entry");
    }
    match entry.proof_kind {
        ProjectAliasProofKind::FilesystemCanonicalization => {
            if payload.get("canonicalized").and_then(Value::as_bool) != Some(true) {
                bail!("filesystem alias proof requires canonicalized=true");
            }
        }
        ProjectAliasProofKind::GitRemote => {
            if payload
                .get("target_remote")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
            {
                bail!("git remote alias proof requires target_remote");
            }
        }
        ProjectAliasProofKind::GitCommitMembership => {
            if payload
                .get("shared_commit_count")
                .and_then(Value::as_u64)
                .is_none_or(|count| count == 0)
            {
                bail!("git commit alias proof requires shared_commit_count > 0");
            }
        }
    }
    if exact_project_id(conn, &entry.canonical_path)?.is_none() {
        bail!("canonical project not found: {}", entry.canonical_path);
    }
    Ok(())
}

fn exact_project_id(conn: &Connection, path: &str) -> Result<Option<i64>> {
    let mut statement =
        conn.prepare("SELECT id FROM projects WHERE project_path = ?1 ORDER BY id LIMIT 2")?;
    let ids = statement
        .query_map([path], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => bail!("canonical project path is ambiguous: {path}"),
    }
}

fn active_alias_target(conn: &Connection, alias_path: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT canonical_project_id FROM project_identity_aliases
         WHERE alias_path = ?1 AND status = 'active'",
        [alias_path],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn validate_digest(value: &str, field: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("{field} must be 64 lowercase hexadecimal characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Result<Connection> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        crate::migrate::run_migrations(&conn)?;
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
        Ok(conn)
    }

    fn entry(alias: &str, target: &str) -> ProjectAliasPlanEntry {
        let payload = serde_json::json!({
            "from_path": alias,
            "to_path": target,
            "target_remote": "github.com/o/r",
            "shared_commit_count": 2
        });
        ProjectAliasPlanEntry {
            alias_path: alias.to_string(),
            canonical_path: target.to_string(),
            proof_kind: ProjectAliasProofKind::GitCommitMembership,
            proof_sha256: proof_sha256(&payload).unwrap(),
            proof_payload: payload,
        }
    }

    fn request<'a>(entries: &'a [ProjectAliasPlanEntry]) -> ProjectAliasApplyRequest<'a> {
        ProjectAliasApplyRequest {
            source_inventory_sha256:
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            actor: "test",
            reason: "fixture",
            now_epoch: 10,
            entries,
        }
    }

    #[test]
    fn apply_and_resolve_alias_without_rewriting_source_rows() -> Result<()> {
        let conn = setup()?;
        conn.execute(
            "INSERT INTO memories(project, scope, memory_type, title, content, status,
                                  created_at_epoch, updated_at_epoch)
             VALUES('/old/repo', 'project', 'decision', 't', 'c', 'active', 1, 1)",
            [],
        )?;
        let entries = [entry("/old/repo", "/new/repo")];
        let result = apply_project_alias_plan(&conn, &request(&entries))?;
        assert_eq!(result.inserted, 1);
        let resolution = resolve_project_identity(&conn, "/old/repo")?;
        assert_eq!(resolution.canonical_path, "/new/repo");
        assert!(resolution.resolved_via_alias);
        assert_eq!(
            project_filter_values(&conn, "/new/repo")?,
            vec!["/new/repo".to_string(), "/old/repo".to_string()]
        );
        let historical: String =
            conn.query_row("SELECT project FROM memories LIMIT 1", [], |row| row.get(0))?;
        assert_eq!(historical, "/old/repo");
        Ok(())
    }

    #[test]
    fn identical_reapply_is_idempotent() -> Result<()> {
        let conn = setup()?;
        let entries = [entry("/old/repo", "/new/repo")];
        apply_project_alias_plan(&conn, &request(&entries))?;
        let result = apply_project_alias_plan(&conn, &request(&entries))?;
        assert_eq!((result.inserted, result.unchanged), (0, 1));
        let events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM project_identity_alias_events",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(events, 1);
        Ok(())
    }

    #[test]
    fn proof_drift_and_collision_fail_closed() -> Result<()> {
        let conn = setup()?;
        let mut bad = entry("/old/repo", "/new/repo");
        bad.proof_sha256 = "b".repeat(64);
        assert!(apply_project_alias_plan(&conn, &request(&[bad])).is_err());

        let good = [entry("/old/repo", "/new/repo")];
        apply_project_alias_plan(&conn, &request(&good))?;
        conn.execute(
            "INSERT INTO workspaces(
                root_path, git_remote, git_branch, created_at_epoch, updated_at_epoch
             ) VALUES('/other/repo', NULL, NULL, 1, 1)",
            [],
        )?;
        let workspace_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO projects(
                workspace_id, project_path, project_key, created_at_epoch, updated_at_epoch
             ) VALUES(?1, '/other/repo', '/other/repo', 1, 1)",
            [workspace_id],
        )?;
        let collision = [entry("/old/repo", "/other/repo")];
        assert!(apply_project_alias_plan(&conn, &request(&collision)).is_err());
        Ok(())
    }

    #[test]
    fn batch_rolls_back_when_later_entry_is_invalid() -> Result<()> {
        let conn = setup()?;
        let entries = [
            entry("/old/repo", "/new/repo"),
            entry("relative", "/new/repo"),
        ];
        assert!(apply_project_alias_plan(&conn, &request(&entries)).is_err());
        let aliases: i64 =
            conn.query_row("SELECT COUNT(*) FROM project_identity_aliases", [], |row| {
                row.get(0)
            })?;
        assert_eq!(aliases, 0);
        Ok(())
    }

    #[test]
    fn project_value_filter_expands_canonical_and_alias_paths() -> Result<()> {
        let conn = setup()?;
        let entries = [entry("/old/repo", "/new/repo")];
        apply_project_alias_plan(&conn, &request(&entries))?;
        let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let (clause, next) =
            push_project_value_filter(&conn, "project", "/new/repo", 1, &mut values)?;
        assert_eq!(clause, "project IN (SELECT value FROM json_each(?1))");
        assert_eq!(next, 2);
        assert_eq!(values.len(), 1);
        Ok(())
    }

    #[test]
    fn resolver_falls_back_on_partial_test_schema() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute("CREATE TABLE memories(project TEXT NOT NULL)", [])?;
        assert_eq!(
            project_filter_values(&conn, "/repo")?,
            vec!["/repo".to_string()]
        );
        Ok(())
    }

    #[test]
    fn capture_retrieval_review_state_and_status_share_alias_identity() -> Result<()> {
        let conn = setup()?;
        let entries = [entry("/old/repo", "/new/repo")];
        apply_project_alias_plan(&conn, &request(&entries))?;

        crate::db::record_captured_event(
            &conn,
            &crate::db::CaptureEventInput {
                host: "codex-cli",
                session_id: "alias-boundary-session",
                project: "/old/repo",
                cwd: None,
                event_type: "user_prompt",
                role: Some("user"),
                tool_name: None,
                content: "alias boundary capture",
                task_kind: None,
            },
        )?;
        let legacy_project_rows: i64 = conn.query_row(
            "SELECT COUNT(*) FROM projects WHERE project_path = '/old/repo'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(legacy_project_rows, 0, "capture must write canonically");

        conn.execute(
            "INSERT INTO memories(
                id, project, scope, memory_type, title, content, status,
                created_at_epoch, updated_at_epoch
             ) VALUES(101, '/old/repo', 'project', 'decision', 'historical', 'body',
                      'active', 1, 1)",
            [],
        )?;
        let retrieved = crate::memory::store::get_recent_project_memories_excluding_types(
            &conn,
            "/new/repo",
            &[],
            10,
        )?;
        assert_eq!(
            retrieved.iter().map(|row| row.id).collect::<Vec<_>>(),
            [101]
        );

        conn.execute(
            "INSERT INTO memory_state_keys(
                owner_scope, owner_key, memory_type, state_key, state_status,
                current_memory_id, created_at_epoch, updated_at_epoch
             ) VALUES('repo', '/old/repo', 'decision', 'alias-state', 'active', 101, 1, 1)",
            [],
        )?;
        assert_eq!(
            crate::memory::state_key::current_memory_id(
                &conn,
                "repo",
                "/new/repo",
                "decision",
                "alias-state",
                2,
            )?,
            Some(101)
        );

        let canonical_project_id = exact_project_id(&conn, "/new/repo")?.unwrap();
        conn.execute(
            "INSERT INTO memory_candidates(
                project_id, source_project, target_project, owner_scope, owner_key,
                scope, memory_type, topic_key, text, evidence_event_ids,
                confidence, risk_class, review_status, created_at_epoch, updated_at_epoch
             ) VALUES(?1, '/old/repo', '/old/repo', 'repo', '/old/repo',
                      'project', 'decision', 'alias-review', 'review me', '[]',
                      0.8, 'medium', 'pending_review', 1, 1)",
            [canonical_project_id],
        )?;
        let pending = crate::memory_candidate::review::list_pending(&conn, Some("/new/repo"), 10)?;
        assert_eq!(pending.len(), 1);

        let top = crate::db::query_top_projects(&conn, 5)?;
        assert_eq!(top[0].project, "/new/repo");
        assert_eq!(top[0].count, 1);
        Ok(())
    }
}
