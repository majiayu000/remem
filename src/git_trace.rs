use anyhow::{bail, Result};
use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use crate::git_util::{short_sha_for, GitCommitMetadata};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitCommitRecord {
    pub id: i64,
    pub project: String,
    pub repo_path: String,
    pub sha: String,
    pub short_sha: String,
    pub branch: Option<String>,
    pub message: Option<String>,
    pub authored_at_epoch: Option<i64>,
    pub changed_files: Vec<String>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSummaryTrace {
    pub request: Option<String>,
    pub completed: Option<String>,
    pub decisions: Option<String>,
    pub learned: Option<String>,
    pub next_steps: Option<String>,
    pub preferences: Option<String>,
    pub created_at_epoch: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitSessionLink {
    pub session_id: String,
    pub memory_session_id: Option<String>,
    pub source: String,
    pub linked_at_epoch: i64,
    pub summary: Option<SessionSummaryTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitLookup {
    pub git: GitCommitRecord,
    pub sessions: Vec<CommitSessionLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCommit {
    pub git: GitCommitRecord,
    pub link: CommitSessionLink,
}

pub struct CommitMetadataInput<'a> {
    pub project: &'a str,
    pub repo_path: Option<&'a str>,
    pub sha: &'a str,
    pub short_sha: Option<&'a str>,
    pub branch: Option<&'a str>,
    pub message: Option<&'a str>,
    pub authored_at_epoch: Option<i64>,
    pub changed_files: &'a [String],
}

pub struct CommitLinkInput<'a> {
    pub metadata: CommitMetadataInput<'a>,
    pub session_id: &'a str,
    pub memory_session_id: Option<&'a str>,
    pub source: &'a str,
}

pub fn upsert_commit_metadata(conn: &Connection, input: &CommitMetadataInput<'_>) -> Result<i64> {
    let sha = normalize_sha(input.sha)?;
    let short_sha = input
        .short_sha
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| short_sha_for(&sha));
    let repo_path = input
        .repo_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(input.project);
    let changed_files = serde_json::to_string(input.changed_files)?;
    let now = chrono::Utc::now().timestamp();

    if sha != short_sha {
        if let Some(id) =
            find_upgradeable_placeholder_commit_id(conn, input.project, &sha, &short_sha)?
        {
            conn.execute(
                "UPDATE git_commits SET
                   repo_path = ?1,
                   sha = ?2,
                   short_sha = ?3,
                   branch = COALESCE(?4, branch),
                   message = COALESCE(?5, message),
                   authored_at_epoch = COALESCE(?6, authored_at_epoch),
                   changed_files = CASE WHEN ?7 != '[]' THEN ?7 ELSE changed_files END,
                   updated_at_epoch = ?8
                 WHERE id = ?9",
                params![
                    repo_path,
                    sha,
                    short_sha,
                    input.branch,
                    input.message,
                    input.authored_at_epoch,
                    changed_files,
                    now,
                    id
                ],
            )?;
            return Ok(id);
        }
    }

    conn.execute(
        "INSERT INTO git_commits
         (project, repo_path, sha, short_sha, branch, message, authored_at_epoch,
          changed_files, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
         ON CONFLICT(project, sha) DO UPDATE SET
           repo_path = excluded.repo_path,
           short_sha = excluded.short_sha,
           branch = COALESCE(excluded.branch, git_commits.branch),
           message = COALESCE(excluded.message, git_commits.message),
           authored_at_epoch = COALESCE(excluded.authored_at_epoch, git_commits.authored_at_epoch),
           changed_files = CASE
             WHEN excluded.changed_files != '[]' THEN excluded.changed_files
             ELSE git_commits.changed_files
           END,
           updated_at_epoch = excluded.updated_at_epoch",
        params![
            input.project,
            repo_path,
            sha,
            short_sha,
            input.branch,
            input.message,
            input.authored_at_epoch,
            changed_files,
            now
        ],
    )?;

    let id = conn.query_row(
        "SELECT id FROM git_commits WHERE project = ?1 AND sha = ?2",
        params![input.project, sha],
        |row| row.get(0),
    )?;
    Ok(id)
}

pub fn link_commit_to_session(conn: &Connection, input: &CommitLinkInput<'_>) -> Result<i64> {
    let session_id = input.session_id.trim();
    if session_id.is_empty() {
        bail!("session_id is required to link a commit");
    }
    let source = input.source.trim();
    if source.is_empty() {
        bail!("source is required to link a commit");
    }

    let commit_id = upsert_commit_metadata(conn, &input.metadata)?;
    link_session_to_commit_id(
        conn,
        commit_id,
        None,
        session_id,
        input.memory_session_id,
        source,
    )?;
    Ok(commit_id)
}

pub fn link_git_metadata_to_session(
    conn: &Connection,
    project: &str,
    session_id: &str,
    memory_session_id: &str,
    metadata: &GitCommitMetadata,
    source: &str,
) -> Result<i64> {
    link_commit_to_session(
        conn,
        &CommitLinkInput {
            metadata: CommitMetadataInput {
                project,
                repo_path: Some(&metadata.repo_path),
                sha: &metadata.sha,
                short_sha: Some(&metadata.short_sha),
                branch: metadata.branch.as_deref(),
                message: metadata.message.as_deref(),
                authored_at_epoch: metadata.authored_at_epoch,
                changed_files: &metadata.changed_files,
            },
            session_id,
            memory_session_id: Some(memory_session_id),
            source,
        },
    )
}

pub fn link_captured_git_metadata_to_session(
    conn: &Connection,
    project: &str,
    session_row_id: i64,
    session_id: &str,
    memory_session_id: &str,
    metadata: &GitCommitMetadata,
) -> Result<i64> {
    validate_capture_session_identity(conn, project, session_row_id, session_id)?;
    let full_sha = validate_full_commit_sha(&metadata.sha)?;
    let short_sha = metadata.short_sha.trim().to_ascii_lowercase();
    if short_sha.len() < 7
        || !short_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !full_sha.starts_with(&short_sha)
    {
        bail!("captured Git metadata short SHA does not match full SHA {full_sha}");
    }
    let commit_id = upsert_commit_metadata(
        conn,
        &CommitMetadataInput {
            project,
            repo_path: Some(&metadata.repo_path),
            sha: &full_sha,
            short_sha: Some(&short_sha),
            branch: metadata.branch.as_deref(),
            message: metadata.message.as_deref(),
            authored_at_epoch: metadata.authored_at_epoch,
            changed_files: &metadata.changed_files,
        },
    )?;
    link_session_to_commit_id(
        conn,
        commit_id,
        Some(session_row_id),
        session_id,
        Some(memory_session_id),
        "capture_git_evidence",
    )?;
    Ok(commit_id)
}

pub fn link_observed_commit_to_session(
    conn: &Connection,
    project: &str,
    session_id: &str,
    memory_session_id: &str,
    commit_sha: &str,
    branch: Option<&str>,
    metadata: Option<&GitCommitMetadata>,
) -> Result<i64> {
    if let Some(metadata) = metadata.filter(|metadata| metadata.matches_sha(commit_sha)) {
        return link_git_metadata_to_session(
            conn,
            project,
            session_id,
            memory_session_id,
            metadata,
            "git_metadata",
        );
    }

    if let Some(commit_id) = find_existing_commit_id(conn, project, commit_sha)? {
        link_session_to_commit_id(
            conn,
            commit_id,
            None,
            session_id,
            Some(memory_session_id),
            "observations",
        )?;
        return Ok(commit_id);
    }

    let changed_files = Vec::new();
    link_commit_to_session(
        conn,
        &CommitLinkInput {
            metadata: CommitMetadataInput {
                project,
                repo_path: Some(project),
                sha: commit_sha,
                short_sha: None,
                branch,
                message: None,
                authored_at_epoch: None,
                changed_files: &changed_files,
            },
            session_id,
            memory_session_id: Some(memory_session_id),
            source: "observations",
        },
    )
}

pub fn link_observed_commits_for_session(
    conn: &Connection,
    project: &str,
    session_id: &str,
    memory_session_id: &str,
) -> Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT commit_sha, branch
         FROM observations
         WHERE project = ?1
           AND memory_session_id = ?2
           AND commit_sha IS NOT NULL
           AND length(trim(commit_sha)) > 0
         GROUP BY commit_sha, branch",
    )?;
    let rows = stmt.query_map(params![project, memory_session_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let observed = rows.collect::<rusqlite::Result<Vec<_>>>()?;

    for (commit_sha, branch) in &observed {
        link_observed_commit_to_session(
            conn,
            project,
            session_id,
            memory_session_id,
            commit_sha,
            branch.as_deref(),
            None,
        )?;
    }

    Ok(observed.len())
}

fn find_upgradeable_placeholder_commit_id(
    conn: &Connection,
    project: &str,
    full_sha: &str,
    short_sha: &str,
) -> Result<Option<i64>> {
    let mut stmt = conn.prepare(
        "SELECT id
         FROM git_commits
         WHERE project = ?1
           AND sha != ?2
           AND (
             sha = ?3
             OR short_sha = ?3
             OR ?2 LIKE sha || '%'
             OR ?2 LIKE short_sha || '%'
           )
         ORDER BY
           CASE
             WHEN sha = ?3 THEN 0
             WHEN short_sha = ?3 THEN 1
             WHEN ?2 LIKE sha || '%' THEN 2
             ELSE 3
           END,
           length(sha) DESC,
           updated_at_epoch DESC
         LIMIT 2",
    )?;
    let ids = stmt
        .query_map(params![project, full_sha, short_sha], |row| {
            row.get::<_, i64>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => bail!("ambiguous commit SHA placeholder: {full_sha}"),
    }
}

fn find_existing_commit_id(
    conn: &Connection,
    project: &str,
    sha_or_prefix: &str,
) -> Result<Option<i64>> {
    let needle = normalize_sha(sha_or_prefix)?;
    let prefix = format!("{needle}%");
    let mut stmt = conn.prepare(
        "SELECT id
         FROM git_commits
         WHERE project = ?1
           AND (
             sha = ?2
             OR short_sha = ?2
             OR sha LIKE ?3
             OR short_sha LIKE ?3
             OR ?2 LIKE sha || '%'
             OR ?2 LIKE short_sha || '%'
           )
         ORDER BY CASE WHEN sha = ?2 THEN 0 WHEN short_sha = ?2 THEN 1 ELSE 2 END,
                  updated_at_epoch DESC
         LIMIT 2",
    )?;
    let ids = stmt
        .query_map(params![project, needle, prefix], |row| row.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match ids.as_slice() {
        [] => Ok(None),
        [id] => Ok(Some(*id)),
        _ => bail!("ambiguous commit SHA prefix: {sha_or_prefix}"),
    }
}

fn link_session_to_commit_id(
    conn: &Connection,
    commit_id: i64,
    session_row_id: Option<i64>,
    session_id: &str,
    memory_session_id: Option<&str>,
    source: &str,
) -> Result<()> {
    let now = chrono::Utc::now().timestamp();
    if let Some(session_row_id) = session_row_id {
        promote_exact_legacy_link(
            conn,
            commit_id,
            session_row_id,
            session_id,
            memory_session_id,
        )?;
        conn.execute(
            "INSERT INTO git_commit_sessions
             (commit_id, session_row_id, session_id, memory_session_id, source, linked_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(commit_id, session_row_id) WHERE session_row_id IS NOT NULL DO UPDATE SET
               session_id = excluded.session_id,
               memory_session_id = COALESCE(excluded.memory_session_id, git_commit_sessions.memory_session_id),
               source = CASE
                 WHEN git_commit_sessions.source IN ('git_metadata', 'capture_git_evidence')
                   THEN git_commit_sessions.source
                 WHEN excluded.source IN ('git_metadata', 'capture_git_evidence')
                   THEN excluded.source
                 ELSE excluded.source
               END",
            params![
                commit_id,
                session_row_id,
                session_id,
                memory_session_id,
                source,
                now
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO git_commit_sessions
             (commit_id, session_row_id, session_id, memory_session_id, source, linked_at_epoch)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5)
             ON CONFLICT(commit_id, session_id) WHERE session_row_id IS NULL DO UPDATE SET
               memory_session_id = COALESCE(excluded.memory_session_id, git_commit_sessions.memory_session_id),
               source = CASE
                 WHEN git_commit_sessions.source IN ('git_metadata', 'capture_git_evidence')
                   THEN git_commit_sessions.source
                 WHEN excluded.source IN ('git_metadata', 'capture_git_evidence')
                   THEN excluded.source
                 ELSE excluded.source
               END",
            params![commit_id, session_id, memory_session_id, source, now],
        )?;
    }
    Ok(())
}

fn promote_exact_legacy_link(
    conn: &Connection,
    commit_id: i64,
    session_row_id: i64,
    session_id: &str,
    memory_session_id: Option<&str>,
) -> Result<()> {
    let expected_memory_session_id = format!("capture-rollup-{session_row_id}");
    if memory_session_id != Some(expected_memory_session_id.as_str()) {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM git_commit_sessions
         WHERE commit_id = ?1
           AND session_row_id IS NULL
           AND session_id = ?2
           AND memory_session_id = ?3
           AND EXISTS (
             SELECT 1 FROM git_commit_sessions durable
             WHERE durable.commit_id = ?1 AND durable.session_row_id = ?4
           )",
        params![
            commit_id,
            session_id,
            expected_memory_session_id,
            session_row_id
        ],
    )?;
    conn.execute(
        "UPDATE git_commit_sessions
         SET session_row_id = ?4
         WHERE commit_id = ?1
           AND session_row_id IS NULL
           AND session_id = ?2
           AND memory_session_id = ?3",
        params![
            commit_id,
            session_id,
            expected_memory_session_id,
            session_row_id
        ],
    )?;
    Ok(())
}

pub fn lookup_commit(
    conn: &Connection,
    project: Option<&str>,
    sha_or_prefix: &str,
) -> Result<Vec<CommitLookup>> {
    let needle = normalize_sha(sha_or_prefix)?;
    let prefix = format!("{needle}%");
    let commits = if let Some(project) = project {
        let mut stmt = conn.prepare(
            "SELECT id, project, repo_path, sha, short_sha, branch, message,
                    authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch
             FROM git_commits
             WHERE project = ?1
               AND (sha = ?2 OR short_sha = ?2 OR sha LIKE ?3 OR short_sha LIKE ?3)
             ORDER BY CASE WHEN sha = ?2 THEN 0 WHEN short_sha = ?2 THEN 1 ELSE 2 END,
                      updated_at_epoch DESC
             LIMIT 20",
        )?;
        let rows = stmt.query_map(params![project, needle, prefix], commit_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    } else {
        let mut stmt = conn.prepare(
            "SELECT id, project, repo_path, sha, short_sha, branch, message,
                    authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch
             FROM git_commits
             WHERE sha = ?1 OR short_sha = ?1 OR sha LIKE ?2 OR short_sha LIKE ?2
             ORDER BY CASE WHEN sha = ?1 THEN 0 WHEN short_sha = ?1 THEN 1 ELSE 2 END,
                      updated_at_epoch DESC
             LIMIT 20",
        )?;
        let rows = stmt.query_map(params![needle, prefix], commit_from_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };

    commits
        .into_iter()
        .map(|git| {
            let sessions = linked_sessions_for_commit(conn, git.id, &git.project)?;
            Ok(CommitLookup { git, sessions })
        })
        .collect()
}

pub fn commits_for_session(
    conn: &Connection,
    project: Option<&str>,
    session_id: &str,
    limit: i64,
) -> Result<Vec<SessionCommit>> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        bail!("session_id is required");
    }
    let limit = limit.clamp(1, 100);
    if let Some(project) = project {
        query_session_commits(
            conn,
            "WHERE c.project = ?1 AND (l.session_id = ?2 OR l.memory_session_id = ?2)",
            params![project, session_id, limit],
        )
    } else {
        query_session_commits(
            conn,
            "WHERE l.session_id = ?1 OR l.memory_session_id = ?1",
            params![session_id, limit],
        )
    }
}

fn query_session_commits<P>(
    conn: &Connection,
    where_clause: &str,
    params: P,
) -> Result<Vec<SessionCommit>>
where
    P: rusqlite::Params,
{
    let sql = format!(
        "SELECT c.id, c.project, c.repo_path, c.sha, c.short_sha, c.branch, c.message,
                c.authored_at_epoch, c.changed_files, c.created_at_epoch, c.updated_at_epoch,
                l.session_id, l.memory_session_id, l.source, l.linked_at_epoch,
                ss.request, ss.completed, ss.decisions, ss.learned, ss.next_steps,
                ss.preferences, ss.created_at_epoch
         FROM git_commit_sessions l
         JOIN git_commits c ON c.id = l.commit_id
         LEFT JOIN session_summaries ss ON ss.id = (
           SELECT latest.id
           FROM session_summaries latest
           WHERE latest.memory_session_id = l.memory_session_id
             AND latest.project = c.project
           ORDER BY COALESCE(latest.covered_to_event_id, 0) DESC,
                    latest.created_at_epoch DESC,
                    latest.id DESC
           LIMIT 1
         )
         {where_clause}
         ORDER BY COALESCE(c.authored_at_epoch, c.updated_at_epoch) DESC, c.id DESC
         LIMIT ?{}",
        if where_clause.contains("?2") {
            "3"
        } else {
            "2"
        }
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params, |row| {
        Ok(SessionCommit {
            git: commit_from_row(row)?,
            link: link_from_row(row, 11)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn linked_sessions_for_commit(
    conn: &Connection,
    commit_id: i64,
    project: &str,
) -> Result<Vec<CommitSessionLink>> {
    let mut stmt = conn.prepare(
        "SELECT l.session_id, l.memory_session_id, l.source, l.linked_at_epoch,
                ss.request, ss.completed, ss.decisions, ss.learned, ss.next_steps,
                ss.preferences, ss.created_at_epoch
         FROM git_commit_sessions l
         LEFT JOIN session_summaries ss ON ss.id = (
           SELECT latest.id
           FROM session_summaries latest
           WHERE latest.memory_session_id = l.memory_session_id
             AND latest.project = ?2
           ORDER BY COALESCE(latest.covered_to_event_id, 0) DESC,
                    latest.created_at_epoch DESC,
                    latest.id DESC
           LIMIT 1
         )
         WHERE l.commit_id = ?1
         ORDER BY l.linked_at_epoch DESC",
    )?;
    let rows = stmt.query_map(params![commit_id, project], |row| link_from_row(row, 0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn normalize_sha(raw: &str) -> Result<String> {
    let sha = raw.trim();
    if sha.is_empty() {
        bail!("commit SHA is required");
    }
    Ok(sha.to_string())
}

fn validate_full_commit_sha(raw: &str) -> Result<String> {
    let sha = raw.trim().to_ascii_lowercase();
    if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("captured commit SHA must be a full 40- or 64-character hexadecimal hash");
    }
    Ok(sha)
}

fn validate_capture_session_identity(
    conn: &Connection,
    project: &str,
    session_row_id: i64,
    session_id: &str,
) -> Result<()> {
    let identity = conn
        .query_row(
            "SELECT sessions.session_id, projects.project_path
             FROM sessions
             JOIN projects ON projects.id = sessions.project_id
             WHERE sessions.id = ?1",
            [session_row_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((stored_session_id, stored_project)) = identity else {
        bail!("captured commit session_row_id {session_row_id} does not exist");
    };
    if stored_session_id != session_id || stored_project != project {
        bail!(
            "captured commit identity mismatch for session_row_id {session_row_id}: expected project={project} session={session_id}, stored project={stored_project} session={stored_session_id}"
        );
    }
    Ok(())
}

fn commit_from_row(row: &Row<'_>) -> rusqlite::Result<GitCommitRecord> {
    Ok(GitCommitRecord {
        id: row.get(0)?,
        project: row.get(1)?,
        repo_path: row.get(2)?,
        sha: row.get(3)?,
        short_sha: row.get(4)?,
        branch: row.get(5)?,
        message: row.get(6)?,
        authored_at_epoch: row.get(7)?,
        changed_files: changed_files_from_row(row, 8)?,
        created_at_epoch: row.get(9)?,
        updated_at_epoch: row.get(10)?,
    })
}

fn link_from_row(row: &Row<'_>, offset: usize) -> rusqlite::Result<CommitSessionLink> {
    let summary = summary_from_row(row, offset + 4)?;
    Ok(CommitSessionLink {
        session_id: row.get(offset)?,
        memory_session_id: row.get(offset + 1)?,
        source: row.get(offset + 2)?,
        linked_at_epoch: row.get(offset + 3)?,
        summary,
    })
}

fn summary_from_row(row: &Row<'_>, offset: usize) -> rusqlite::Result<Option<SessionSummaryTrace>> {
    let summary = SessionSummaryTrace {
        request: row.get(offset)?,
        completed: row.get(offset + 1)?,
        decisions: row.get(offset + 2)?,
        learned: row.get(offset + 3)?,
        next_steps: row.get(offset + 4)?,
        preferences: row.get(offset + 5)?,
        created_at_epoch: row.get(offset + 6)?,
    };
    if summary.request.is_none()
        && summary.completed.is_none()
        && summary.decisions.is_none()
        && summary.learned.is_none()
        && summary.next_steps.is_none()
        && summary.preferences.is_none()
        && summary.created_at_epoch.is_none()
    {
        Ok(None)
    } else {
        Ok(Some(summary))
    }
}

fn changed_files_from_row(row: &Row<'_>, idx: usize) -> rusqlite::Result<Vec<String>> {
    let raw: String = row.get(idx)?;
    serde_json::from_str(&raw)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(idx, Type::Text, Box::new(err)))
}

#[cfg(test)]
mod tests;
