use anyhow::Result;
use rusqlite::{params, Connection};

pub(super) const PROJECT: &str = "/tmp/remem-current-contract-eval/repo";
pub(super) const PROMPT_PROJECT: &str = "/tmp/remem-current-contract-eval/prompt";
pub(super) const ABSTAIN_PROJECT: &str = "/tmp/remem-current-contract-eval/abstain";
pub(super) const HOST: &str = "claude-code";
pub(super) const PROMPT_SESSION: &str = "eval-current-contract-prompt";
pub(super) const ABSTAIN_SESSION: &str = "eval-current-contract-abstain";

pub(super) fn setup_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    Ok(conn)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_state_key(
    conn: &Connection,
    id: i64,
    owner_scope: &str,
    owner_key: &str,
    state_key: &str,
    state_status: &str,
    current_memory_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_state_keys
         (id, owner_scope, owner_key, memory_type, state_key, state_label,
          state_status, current_memory_id, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, 'decision', ?4, ?4, ?5, ?6, 1, 10)",
        params![
            id,
            owner_scope,
            owner_key,
            state_key,
            state_status,
            current_memory_id
        ],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_current_state_memory_at(
    conn: &Connection,
    id: i64,
    state_key_id: i64,
    title: &str,
    content: &str,
    status: &str,
    created_at_epoch: i64,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope, source_project,
          target_project, owner_scope, owner_key, context_class, expires_at_epoch,
          valid_from_epoch, valid_to_epoch, state_key_id)
         VALUES (?1, NULL, ?2, NULL, ?3, ?4, 'decision', NULL,
                 ?5, ?5, ?6, NULL, 'project', ?2, ?2, 'repo', ?2,
                 'startup_core', NULL, ?7, ?8, ?9)",
        params![
            id,
            PROJECT,
            title,
            content,
            created_at_epoch,
            status,
            valid_from_epoch,
            valid_to_epoch,
            state_key_id
        ],
    )?;
    Ok(())
}

pub(super) fn set_memory_source(
    conn: &Connection,
    memory_id: i64,
    session_id: &str,
    files: &[&str],
) -> Result<()> {
    let files_json = serde_json::to_string(files)?;
    conn.execute(
        "UPDATE memories
         SET session_id = ?1, files = ?2, branch = 'main'
         WHERE id = ?3",
        params![session_id, files_json, memory_id],
    )?;
    Ok(())
}

pub(super) fn set_current_memory(
    conn: &Connection,
    state_key_id: i64,
    current_memory_id: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE memory_state_keys
         SET current_memory_id = ?1, updated_at_epoch = updated_at_epoch + 1
         WHERE id = ?2",
        params![current_memory_id, state_key_id],
    )?;
    Ok(())
}

pub(super) fn set_memory_files_raw(conn: &Connection, memory_id: i64, files: &str) -> Result<()> {
    conn.execute(
        "UPDATE memories
         SET session_id = 'eval-error-source', files = ?1, branch = 'main'
         WHERE id = ?2",
        params![files, memory_id],
    )?;
    Ok(())
}

pub(super) fn link_current_state_commit(
    conn: &Connection,
    id: i64,
    sha: &str,
    epoch: i64,
    changed_files: &[&str],
    memory_session_id: &str,
) -> Result<()> {
    insert_current_state_commit(conn, id, sha, epoch, changed_files)?;
    conn.execute(
        "INSERT INTO git_commit_sessions
         (commit_id, session_id, memory_session_id, source, linked_at_epoch)
         VALUES (?1, ?2, ?3, 'eval-current-contracts', ?4)",
        params![id, format!("content-{id}"), memory_session_id, epoch],
    )?;
    Ok(())
}

pub(super) fn insert_current_state_commit(
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
         VALUES (?1, ?2, ?2, ?3, ?3, 'main', NULL, ?4, ?5, ?4, ?4)",
        params![id, PROJECT, sha, epoch, changed_files],
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert_fact(
    conn: &Connection,
    id: i64,
    source_memory_id: i64,
    object: &str,
    valid_from_epoch: Option<i64>,
    valid_to_epoch: Option<i64>,
    learned_at_epoch: i64,
    status: &str,
    invalidated_at_epoch: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT INTO memory_facts
         (id, project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
          learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
          confidence, supersedes_fact_id, status, invalidated_at_epoch,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, 'deploy-target', 'affects_project', ?3, ?4, ?5, ?6, ?7,
                 NULL, '[]', 0.95, NULL, ?8, ?9, ?6, ?6)",
        params![
            id,
            PROJECT,
            object,
            valid_from_epoch,
            valid_to_epoch,
            learned_at_epoch,
            source_memory_id,
            status,
            invalidated_at_epoch
        ],
    )?;
    Ok(())
}

pub(super) fn seed_prompt_memory(conn: &Connection) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO memories
         (session_id, project, topic_key, title, content, memory_type, files,
          created_at_epoch, updated_at_epoch, status, branch, scope)
         VALUES ('eval-current-contract-prompt-seed', ?1, 'sqlcipher-storage',
                 'SQLCipher storage decision',
                 'Persist private data with SQLCipher encryption at rest.',
                 'decision', NULL, ?2, ?2, 'active', NULL, 'project')",
        params![PROMPT_PROJECT, now],
    )?;
    Ok(conn.last_insert_rowid())
}
