//! Read-only project path/alias inventory for the G1 governance task.
//!
//! Run against a current-schema database copy:
//! `REMEM_DATA_DIR=/path/to/copy cargo run --example project_alias_inventory`

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use remem::project_alias::{proof_sha256, ProjectAliasProofKind};

const INVENTORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum SurfaceRole {
    Ownership,
    HistoricalOwnership,
    ContextEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct SurfaceCount {
    table: String,
    column: String,
    role: SurfaceRole,
    row_count: i64,
}

#[derive(Debug, Clone, Default)]
struct ObservedPath {
    surfaces: Vec<SurfaceCount>,
    stored_remotes: BTreeSet<String>,
    commit_shas: BTreeSet<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct LiveEvidence {
    exists: bool,
    canonical_root: Option<String>,
    remote: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Classification {
    Exact,
    Moved,
    Missing,
    Ambiguous,
    NonPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct PathInventoryRow {
    path: String,
    classification: Classification,
    canonical_target: Option<String>,
    stored_remotes: Vec<String>,
    live_remote: Option<String>,
    target_remote: Option<String>,
    commit_count: usize,
    shared_commit_count: usize,
    reasons: Vec<String>,
    surfaces: Vec<SurfaceCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct InventorySummary {
    observed_values: usize,
    exact: usize,
    moved: usize,
    missing: usize,
    ambiguous: usize,
    non_path: usize,
    proposed_aliases: usize,
    blocked_ownership_paths: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AliasProposal {
    alias_path: String,
    canonical_path: String,
    proof_kind: ProjectAliasProofKind,
    proof_payload: serde_json::Value,
    proof_sha256: String,
    target_remote: Option<String>,
    shared_commit_count: usize,
    ownership_rows: i64,
    context_evidence_rows: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct BlockedOwnershipPath {
    path: String,
    classification: Classification,
    ownership_rows: i64,
    reasons: Vec<String>,
}

#[derive(Debug, Serialize)]
struct InventoryReport {
    schema_version: u32,
    runtime_version: String,
    database_schema_version: i64,
    sqlite_user_version: i64,
    inventory_sha256: String,
    summary: InventorySummary,
    proposed_aliases: Vec<AliasProposal>,
    blocked_ownership_paths: Vec<BlockedOwnershipPath>,
    paths: Vec<PathInventoryRow>,
}

#[derive(Debug, Clone, Copy)]
struct CandidateColumn {
    name: &'static str,
    role: SurfaceRole,
    owner_scope_guard: bool,
}

const CANDIDATE_COLUMNS: &[CandidateColumn] = &[
    CandidateColumn {
        name: "project",
        role: SurfaceRole::Ownership,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "source_project",
        role: SurfaceRole::Ownership,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "target_project",
        role: SurfaceRole::Ownership,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "project_path",
        role: SurfaceRole::Ownership,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "root_path",
        role: SurfaceRole::Ownership,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "repo_path",
        role: SurfaceRole::ContextEvidence,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "owner_key",
        role: SurfaceRole::Ownership,
        owner_scope_guard: true,
    },
    CandidateColumn {
        name: "legacy_project",
        role: SurfaceRole::HistoricalOwnership,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "cwd",
        role: SurfaceRole::ContextEvidence,
        owner_scope_guard: false,
    },
    CandidateColumn {
        name: "workspace_root",
        role: SurfaceRole::ContextEvidence,
        owner_scope_guard: false,
    },
];

fn main() -> Result<()> {
    let conn = remem::db::open_db_read_only_current()
        .context("open current-schema remem database read-only")?;
    let report = build_report(&conn, resolve_live_evidence, live_repo_contains_commit)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn build_report(
    conn: &Connection,
    resolver: impl Fn(&str) -> LiveEvidence,
    contains_commit: impl Fn(&str, &str) -> bool,
) -> Result<InventoryReport> {
    let sqlite_user_version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let database_schema_version = logical_schema_version(conn, sqlite_user_version)?;
    let observed = load_observed_paths(conn)?;
    let paths = classify_paths(observed, resolver, contains_commit);
    let (proposed_aliases, blocked_ownership_paths) = build_alias_plan(&paths)?;
    let summary = summarize(
        &paths,
        proposed_aliases.len(),
        blocked_ownership_paths.len(),
    );
    let mut report = InventoryReport {
        schema_version: INVENTORY_SCHEMA_VERSION,
        runtime_version: env!("CARGO_PKG_VERSION").to_string(),
        database_schema_version,
        sqlite_user_version,
        inventory_sha256: String::new(),
        summary,
        proposed_aliases,
        blocked_ownership_paths,
        paths,
    };
    let digest_input = serde_json::to_vec(&serde_json::to_value(&report)?)?;
    report.inventory_sha256 = format!("{:x}", Sha256::digest(digest_input));
    Ok(report)
}

fn load_observed_paths(conn: &Connection) -> Result<BTreeMap<String, ObservedPath>> {
    let mut observed = BTreeMap::<String, ObservedPath>::new();
    let mut tables = Vec::new();
    let mut statement = conn.prepare(
        "SELECT name FROM sqlite_schema
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
           AND COALESCE(sql, '') NOT LIKE 'CREATE VIRTUAL TABLE%'
         ORDER BY name",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        tables.push(row?);
    }

    for table in tables {
        let columns = table_columns(conn, &table)?;
        for candidate in CANDIDATE_COLUMNS {
            if !columns.contains(candidate.name) {
                continue;
            }
            if candidate.owner_scope_guard && !columns.contains("owner_scope") {
                continue;
            }
            load_surface_counts(conn, &table, *candidate, &mut observed)?;
        }
    }
    load_workspace_remotes(conn, &mut observed)?;
    load_git_commit_fingerprints(conn, &mut observed)?;
    for value in observed.values_mut() {
        value.surfaces.sort();
    }
    Ok(observed)
}

fn logical_schema_version(conn: &Connection, sqlite_user_version: i64) -> Result<i64> {
    let has_migrations: bool = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema
            WHERE type='table' AND name='_schema_migrations'
        )",
        [],
        |row| row.get(0),
    )?;
    if !has_migrations {
        return Ok(sqlite_user_version);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM _schema_migrations",
        [],
        |row| row.get(0),
    )
    .context("read logical schema version")
}

fn table_columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let mut columns = BTreeSet::new();
    for row in rows {
        columns.insert(row?);
    }
    Ok(columns)
}

fn load_surface_counts(
    conn: &Connection,
    table: &str,
    candidate: CandidateColumn,
    observed: &mut BTreeMap<String, ObservedPath>,
) -> Result<()> {
    let table_sql = quote_identifier(table);
    let column_sql = quote_identifier(candidate.name);
    let owner_guard = if candidate.owner_scope_guard {
        " AND owner_scope IN ('repo', 'workspace')"
    } else {
        ""
    };
    let sql = format!(
        "SELECT {column_sql}, COUNT(*) FROM {table_sql}
         WHERE {column_sql} IS NOT NULL
           AND typeof({column_sql}) = 'text'
           AND trim({column_sql}) <> ''{owner_guard}
         GROUP BY {column_sql}
         ORDER BY {column_sql}"
    );
    let mut statement = conn
        .prepare(&sql)
        .with_context(|| format!("prepare inventory query for {table}.{}", candidate.name))?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (path, row_count) = row?;
        observed
            .entry(path)
            .or_default()
            .surfaces
            .push(SurfaceCount {
                table: table.to_string(),
                column: candidate.name.to_string(),
                role: candidate.role,
                row_count,
            });
    }
    Ok(())
}

fn load_workspace_remotes(
    conn: &Connection,
    observed: &mut BTreeMap<String, ObservedPath>,
) -> Result<()> {
    let has_workspaces: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='workspaces')",
        [],
        |row| row.get(0),
    )?;
    if !has_workspaces {
        return Ok(());
    }
    let columns = table_columns(conn, "workspaces")?;
    if !columns.contains("root_path") || !columns.contains("git_remote") {
        return Ok(());
    }
    let mut statement = conn.prepare(
        "SELECT root_path, git_remote FROM workspaces
         WHERE root_path IS NOT NULL AND trim(root_path) <> ''
         ORDER BY root_path, git_remote",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (path, remote) = row?;
        let value = observed.entry(path).or_default();
        if let Some(remote) = remote.and_then(|value| normalize_remote(&value)) {
            value.stored_remotes.insert(remote);
        }
    }
    Ok(())
}

fn load_git_commit_fingerprints(
    conn: &Connection,
    observed: &mut BTreeMap<String, ObservedPath>,
) -> Result<()> {
    let has_commits: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='git_commits')",
        [],
        |row| row.get(0),
    )?;
    if !has_commits {
        return Ok(());
    }
    let columns = table_columns(conn, "git_commits")?;
    if !columns.contains("project") || !columns.contains("sha") {
        return Ok(());
    }
    let mut statement = conn.prepare(
        "SELECT project, sha FROM git_commits
         WHERE project IS NOT NULL AND trim(project) <> ''
           AND sha IS NOT NULL AND trim(sha) <> ''
         ORDER BY project, sha",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (project, sha) = row?;
        observed.entry(project).or_default().commit_shas.insert(sha);
    }
    Ok(())
}

fn classify_paths(
    observed: BTreeMap<String, ObservedPath>,
    resolver: impl Fn(&str) -> LiveEvidence,
    contains_commit: impl Fn(&str, &str) -> bool,
) -> Vec<PathInventoryRow> {
    let mut live = BTreeMap::<String, LiveEvidence>::new();
    for path in observed.keys() {
        if Path::new(path).is_absolute() {
            live.insert(path.clone(), resolver(path));
        }
    }

    let mut remote_targets = BTreeMap::<String, BTreeSet<String>>::new();
    for (path, evidence) in &live {
        if !evidence.exists {
            continue;
        }
        let target = evidence
            .canonical_root
            .clone()
            .unwrap_or_else(|| path.clone());
        if let Some(remote) = &evidence.remote {
            remote_targets
                .entry(remote.clone())
                .or_default()
                .insert(target);
        }
    }

    let mut rows = Vec::new();
    for (path, data) in &observed {
        let stored_remotes = data.stored_remotes.iter().cloned().collect::<Vec<_>>();
        if !Path::new(path).is_absolute() {
            rows.push(PathInventoryRow {
                path: path.clone(),
                classification: Classification::NonPath,
                canonical_target: None,
                stored_remotes,
                live_remote: None,
                target_remote: None,
                commit_count: data.commit_shas.len(),
                shared_commit_count: 0,
                reasons: vec!["value_is_not_an_absolute_path".to_string()],
                surfaces: data.surfaces.clone(),
            });
            continue;
        }

        let evidence = live.get(path).cloned().unwrap_or_default();
        let stored_set = stored_remotes.iter().cloned().collect::<BTreeSet<_>>();
        let live_remote = evidence.remote.clone();
        let conflicting_remote = live_remote
            .as_ref()
            .is_some_and(|remote| !stored_set.is_empty() && !stored_set.contains(remote));

        let mut shared_commit_count = 0;
        let (classification, canonical_target, reasons) = if evidence.exists {
            if conflicting_remote {
                (
                    Classification::Ambiguous,
                    evidence.canonical_root.clone(),
                    vec!["stored_and_live_git_remote_conflict".to_string()],
                )
            } else if evidence.canonical_root.as_deref() == Some(path.as_str()) {
                (
                    Classification::Exact,
                    Some(path.clone()),
                    vec!["path_exists_and_matches_canonical_git_root".to_string()],
                )
            } else {
                (
                    Classification::Moved,
                    evidence.canonical_root.clone(),
                    vec!["path_exists_but_resolves_to_a_different_canonical_root".to_string()],
                )
            }
        } else {
            let mut candidates = stored_remotes
                .iter()
                .flat_map(|remote| remote_targets.get(remote).into_iter().flatten().cloned())
                .collect::<BTreeSet<_>>();
            let remote_candidate_count = candidates.len();
            for (live_path, live_evidence) in &live {
                if !live_evidence.exists {
                    continue;
                }
                let Some(live_data) = observed.get(live_path) else {
                    continue;
                };
                let shared = data
                    .commit_shas
                    .intersection(&live_data.commit_shas)
                    .count();
                if shared == 0 {
                    continue;
                }
                shared_commit_count = shared_commit_count.max(shared);
                candidates.insert(
                    live_evidence
                        .canonical_root
                        .clone()
                        .unwrap_or_else(|| live_path.clone()),
                );
            }
            if !data.commit_shas.is_empty() {
                let missing_name = Path::new(path).file_name();
                for (live_path, live_evidence) in &live {
                    if !live_evidence.exists {
                        continue;
                    }
                    let target = live_evidence
                        .canonical_root
                        .as_deref()
                        .unwrap_or(live_path.as_str());
                    if missing_name.is_none() || Path::new(target).file_name() != missing_name {
                        continue;
                    }
                    let matching = data
                        .commit_shas
                        .iter()
                        .filter(|sha| contains_commit(target, sha))
                        .count();
                    if matching == 0 {
                        continue;
                    }
                    shared_commit_count = shared_commit_count.max(matching);
                    candidates.insert(target.to_string());
                }
            }
            match candidates.len() {
                1 => (
                    Classification::Moved,
                    candidates.into_iter().next(),
                    vec![if shared_commit_count > 0 {
                        "missing_path_commits_resolve_in_one_live_repository".to_string()
                    } else {
                        "missing_path_has_one_live_target_with_matching_git_remote".to_string()
                    }],
                ),
                0 => (
                    Classification::Missing,
                    None,
                    vec!["path_missing_without_a_unique_git_remote_target".to_string()],
                ),
                _ => (
                    Classification::Ambiguous,
                    None,
                    vec![if remote_candidate_count > 1 {
                        "git_remote_matches_multiple_live_worktree_roots".to_string()
                    } else {
                        "identity_evidence_matches_multiple_live_roots".to_string()
                    }],
                ),
            }
        };
        let target_remote = canonical_target.as_deref().and_then(|target| {
            live.values()
                .find(|candidate| candidate.canonical_root.as_deref() == Some(target))
                .and_then(|candidate| candidate.remote.clone())
        });
        rows.push(PathInventoryRow {
            path: path.clone(),
            classification,
            canonical_target,
            stored_remotes,
            live_remote,
            target_remote,
            commit_count: data.commit_shas.len(),
            shared_commit_count,
            reasons,
            surfaces: data.surfaces.clone(),
        });
    }
    rows
}

fn resolve_live_evidence(value: &str) -> LiveEvidence {
    let path = Path::new(value);
    if !path.exists() {
        return LiveEvidence::default();
    }
    let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let canonical_root = remem::git_util::resolve_toplevel(&canonical_path)
        .and_then(|root| std::fs::canonicalize(&root).ok().or(Some(root)))
        .unwrap_or(canonical_path);
    LiveEvidence {
        exists: true,
        canonical_root: Some(canonical_root.to_string_lossy().to_string()),
        remote: git_origin(&canonical_root).and_then(|remote| normalize_remote(&remote)),
    }
}

fn git_origin(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn live_repo_contains_commit(root: &str, sha: &str) -> bool {
    if sha.is_empty() || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    Command::new("git")
        .args(["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .current_dir(root)
        .status()
        .is_ok_and(|status| status.success())
}

fn normalize_remote(value: &str) -> Option<String> {
    let mut remote = value.trim().trim_end_matches('/').to_string();
    if remote.is_empty() {
        return None;
    }
    if let Some(rest) = remote.strip_prefix("git@") {
        if let Some((host, path)) = rest.split_once(':') {
            remote = format!("{host}/{path}");
        }
    } else if let Some((_, rest)) = remote.split_once("://") {
        remote = rest.to_string();
        if let Some((_, without_auth)) = remote.rsplit_once('@') {
            remote = without_auth.to_string();
        }
    } else if let Some(rest) = remote.strip_prefix("ssh://git@") {
        remote = rest.to_string();
    }
    remote = remote
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_string();
    let (host, path) = remote.split_once('/')?;
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{}/{}", host.to_ascii_lowercase(), path))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn ownership_row_counts(path: &PathInventoryRow) -> (i64, i64) {
    path.surfaces.iter().fold((0, 0), |mut counts, surface| {
        if surface.role == SurfaceRole::ContextEvidence {
            counts.1 += surface.row_count;
        } else {
            counts.0 += surface.row_count;
        }
        counts
    })
}

fn build_alias_plan(
    paths: &[PathInventoryRow],
) -> Result<(Vec<AliasProposal>, Vec<BlockedOwnershipPath>)> {
    let mut proposals = Vec::new();
    let mut blocked = Vec::new();
    let canonical_project_paths = paths
        .iter()
        .filter(|path| {
            path.surfaces
                .iter()
                .any(|surface| surface.table == "projects" && surface.column == "project_path")
        })
        .map(|path| path.path.as_str())
        .collect::<BTreeSet<_>>();
    for path in paths {
        let (ownership_rows, context_evidence_rows) = ownership_row_counts(path);
        if ownership_rows == 0 {
            continue;
        }
        match path.classification {
            Classification::Moved => {
                let Some(target) = path.canonical_target.as_ref() else {
                    blocked.push(BlockedOwnershipPath {
                        path: path.path.clone(),
                        classification: Classification::Ambiguous,
                        ownership_rows,
                        reasons: vec!["moved_path_missing_canonical_target".to_string()],
                    });
                    continue;
                };
                if !canonical_project_paths.contains(target.as_str()) {
                    blocked.push(BlockedOwnershipPath {
                        path: path.path.clone(),
                        classification: Classification::Ambiguous,
                        ownership_rows,
                        reasons: vec!["canonical_target_missing_projects_row".to_string()],
                    });
                    continue;
                }
                let proof_kind = if path.shared_commit_count > 0 {
                    ProjectAliasProofKind::GitCommitMembership
                } else if path
                    .stored_remotes
                    .iter()
                    .any(|remote| path.target_remote.as_deref() == Some(remote.as_str()))
                {
                    ProjectAliasProofKind::GitRemote
                } else {
                    ProjectAliasProofKind::FilesystemCanonicalization
                };
                let proof_payload = json!({
                    "from_path": path.path,
                    "to_path": target,
                    "target_remote": path.target_remote,
                    "shared_commit_count": path.shared_commit_count,
                    "canonicalized": proof_kind == ProjectAliasProofKind::FilesystemCanonicalization,
                    "ownership_rows": ownership_rows,
                    "context_evidence_rows": context_evidence_rows
                });
                proposals.push(AliasProposal {
                    alias_path: path.path.clone(),
                    canonical_path: target.clone(),
                    proof_kind,
                    proof_sha256: proof_sha256(&proof_payload)?,
                    proof_payload,
                    target_remote: path.target_remote.clone(),
                    shared_commit_count: path.shared_commit_count,
                    ownership_rows,
                    context_evidence_rows,
                });
            }
            Classification::Missing | Classification::Ambiguous => {
                blocked.push(BlockedOwnershipPath {
                    path: path.path.clone(),
                    classification: path.classification,
                    ownership_rows,
                    reasons: path.reasons.clone(),
                });
            }
            Classification::Exact | Classification::NonPath => {}
        }
    }
    proposals.sort_by(|a, b| {
        (&a.alias_path, &a.canonical_path).cmp(&(&b.alias_path, &b.canonical_path))
    });
    blocked.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((proposals, blocked))
}

fn summarize(
    paths: &[PathInventoryRow],
    proposed_aliases: usize,
    blocked_ownership_paths: usize,
) -> InventorySummary {
    let mut summary = InventorySummary {
        observed_values: paths.len(),
        exact: 0,
        moved: 0,
        missing: 0,
        ambiguous: 0,
        non_path: 0,
        proposed_aliases,
        blocked_ownership_paths,
    };
    for path in paths {
        match path.classification {
            Classification::Exact => summary.exact += 1,
            Classification::Moved => summary.moved += 1,
            Classification::Missing => summary.missing += 1,
            Classification::Ambiguous => summary.ambiguous += 1,
            Classification::NonPath => summary.non_path += 1,
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_normalization_matches_https_and_ssh() {
        assert_eq!(
            normalize_remote("https://github.com/Org/Repo.git"),
            Some("github.com/Org/Repo".to_string())
        );
        assert_eq!(
            normalize_remote("git@github.com:Org/Repo.git"),
            Some("github.com/Org/Repo".to_string())
        );
        assert_eq!(normalize_remote(""), None);
    }

    #[test]
    fn inventory_reads_only_allowlisted_path_columns() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE workspaces(root_path TEXT, git_remote TEXT);
             CREATE TABLE memories(
                 project TEXT,
                 source_project TEXT,
                 owner_scope TEXT,
                 owner_key TEXT,
                 content TEXT
             );
             INSERT INTO workspaces VALUES('/old/repo', 'https://github.com/o/r.git');
             INSERT INTO memories VALUES('/old/repo', '/old/repo', 'repo', '/old/repo', 'secret');
             INSERT INTO memories VALUES('/old/repo', '/old/repo', 'user', 'not-a-path', 'secret');",
        )?;
        let observed = load_observed_paths(&conn)?;
        let old = observed.get("/old/repo").expect("old path");
        assert_eq!(old.stored_remotes.len(), 1);
        assert!(old.surfaces.iter().any(|s| s.column == "project"));
        assert!(old.surfaces.iter().any(|s| s.column == "owner_key"));
        assert!(!observed.contains_key("not-a-path"));
        assert!(!observed.contains_key("secret"));
        Ok(())
    }

    #[test]
    fn missing_path_with_one_remote_match_is_moved() {
        let mut observed = BTreeMap::new();
        observed.insert(
            "/old/repo".to_string(),
            ObservedPath {
                surfaces: Vec::new(),
                stored_remotes: BTreeSet::from(["github.com/o/r".to_string()]),
                commit_shas: BTreeSet::new(),
            },
        );
        observed.insert("/new/repo".to_string(), ObservedPath::default());
        let rows = classify_paths(
            observed,
            |path| match path {
                "/new/repo" => LiveEvidence {
                    exists: true,
                    canonical_root: Some("/new/repo".to_string()),
                    remote: Some("github.com/o/r".to_string()),
                },
                _ => LiveEvidence::default(),
            },
            |_, _| false,
        );
        let old = rows.iter().find(|row| row.path == "/old/repo").unwrap();
        assert_eq!(old.classification, Classification::Moved);
        assert_eq!(old.canonical_target.as_deref(), Some("/new/repo"));
    }

    #[test]
    fn multiple_live_roots_for_remote_abstain_as_ambiguous() {
        let mut observed = BTreeMap::new();
        observed.insert(
            "/old/repo".to_string(),
            ObservedPath {
                surfaces: Vec::new(),
                stored_remotes: BTreeSet::from(["github.com/o/r".to_string()]),
                commit_shas: BTreeSet::new(),
            },
        );
        observed.insert("/worktree/a".to_string(), ObservedPath::default());
        observed.insert("/worktree/b".to_string(), ObservedPath::default());
        let rows = classify_paths(
            observed,
            |path| {
                if path.starts_with("/worktree/") {
                    LiveEvidence {
                        exists: true,
                        canonical_root: Some(path.to_string()),
                        remote: Some("github.com/o/r".to_string()),
                    }
                } else {
                    LiveEvidence::default()
                }
            },
            |_, _| false,
        );
        let old = rows.iter().find(|row| row.path == "/old/repo").unwrap();
        assert_eq!(old.classification, Classification::Ambiguous);
        assert!(old.canonical_target.is_none());
    }

    #[test]
    fn shared_commit_evidence_links_missing_path_without_stored_remote() {
        let mut observed = BTreeMap::new();
        observed.insert(
            "/old/repo".to_string(),
            ObservedPath {
                surfaces: Vec::new(),
                stored_remotes: BTreeSet::new(),
                commit_shas: BTreeSet::from(["abc123".to_string()]),
            },
        );
        observed.insert(
            "/new/repo".to_string(),
            ObservedPath {
                surfaces: Vec::new(),
                stored_remotes: BTreeSet::new(),
                commit_shas: BTreeSet::from(["abc123".to_string(), "def456".to_string()]),
            },
        );
        let rows = classify_paths(
            observed,
            |path| match path {
                "/new/repo" => LiveEvidence {
                    exists: true,
                    canonical_root: Some("/new/repo".to_string()),
                    remote: Some("github.com/o/r".to_string()),
                },
                _ => LiveEvidence::default(),
            },
            |_, _| false,
        );
        let old = rows.iter().find(|row| row.path == "/old/repo").unwrap();
        assert_eq!(old.classification, Classification::Moved);
        assert_eq!(old.canonical_target.as_deref(), Some("/new/repo"));
        assert_eq!(old.shared_commit_count, 1);
    }

    #[test]
    fn live_repository_commit_proof_links_same_name_missing_path() {
        let mut observed = BTreeMap::new();
        observed.insert(
            "/old/parent/repo".to_string(),
            ObservedPath {
                surfaces: Vec::new(),
                stored_remotes: BTreeSet::new(),
                commit_shas: BTreeSet::from(["abc123".to_string()]),
            },
        );
        observed.insert("/new/parent/repo".to_string(), ObservedPath::default());
        let rows = classify_paths(
            observed,
            |path| match path {
                "/new/parent/repo" => LiveEvidence {
                    exists: true,
                    canonical_root: Some("/new/parent/repo".to_string()),
                    remote: Some("github.com/o/r".to_string()),
                },
                _ => LiveEvidence::default(),
            },
            |root, sha| root == "/new/parent/repo" && sha == "abc123",
        );
        let old = rows
            .iter()
            .find(|row| row.path == "/old/parent/repo")
            .unwrap();
        assert_eq!(old.classification, Classification::Moved);
        assert_eq!(old.canonical_target.as_deref(), Some("/new/parent/repo"));
        assert_eq!(old.shared_commit_count, 1);
    }

    #[test]
    fn report_digest_is_deterministic() -> Result<()> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA user_version = 80;
             CREATE TABLE workspaces(root_path TEXT, git_remote TEXT);
             INSERT INTO workspaces VALUES('/repo', 'https://github.com/o/r.git');",
        )?;
        let resolver = |path: &str| LiveEvidence {
            exists: true,
            canonical_root: Some(path.to_string()),
            remote: Some("github.com/o/r".to_string()),
        };
        let first = build_report(&conn, resolver, |_, _| false)?;
        let second = build_report(&conn, resolver, |_, _| false)?;
        assert_eq!(first.inventory_sha256, second.inventory_sha256);
        assert_eq!(first.paths, second.paths);
        Ok(())
    }
}
