use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use super::types::NormalizedEvent;
use crate::identity::{InstallHost, ProjectKey, SessionId, WorkspaceKey};

/// `content_text` stays inline up to 16 KiB; bigger payloads spill into
/// `event_blobs` while keeping a prefix/suffix summary in the event row.
const MAX_CONTENT_TEXT_BYTES: usize = 16 * 1024;

/// Insert one normalized event into `captured_events`, creating any missing
/// hosts / workspaces / projects / sessions rows along the way. Oversize
/// payloads spill into `event_blobs`; `content_text` keeps
/// a prefix + suffix summary so the row is still useful for FTS / preview.
/// Idempotent on `(host_id, session_id, event_id)`.
pub fn insert_captured_event(conn: &Connection, ev: &NormalizedEvent) -> Result<i64> {
    let host_id = lookup_host_id(conn, ev.identity.host)?;
    let workspace_id = ensure_workspace(conn, &ev.identity.workspace, ev.created_at_epoch)?;
    let project_id = ensure_project(
        conn,
        workspace_id,
        &ev.identity.project,
        ev.created_at_epoch,
    )?;
    let session_row_id = ensure_session(
        conn,
        host_id,
        workspace_id,
        project_id,
        &ev.identity.session_id,
        ev.created_at_epoch,
    )?;

    let storage = resolve_storage(conn, ev.content_text.as_deref(), ev.created_at_epoch)?;
    let retention_class = match storage.kind {
        StorageKind::Empty | StorageKind::Inline => ev.retention_class.clone(),
        StorageKind::Spilled => "raw_compact".to_string(),
    };

    conn.execute(
        "INSERT INTO captured_events(
            host_id, workspace_id, project_id, session_row_id, session_id,
            turn_id, event_id, event_type, role, tool_name,
            content_text, content_blob_id, content_hash,
            token_estimate, retention_class,
            created_at_epoch, inserted_at_epoch
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5,
            ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13,
            ?14, ?15,
            ?16, ?16
         )
         ON CONFLICT(host_id, session_id, event_id) DO NOTHING",
        params![
            host_id,
            workspace_id,
            project_id,
            session_row_id,
            ev.identity.session_id.0,
            ev.identity.turn_id.as_ref().map(|t| t.0.as_str()),
            ev.identity.event_id.0,
            ev.event_type,
            ev.role,
            ev.tool_name,
            storage.content_text,
            storage.content_blob_id,
            storage.content_hash,
            ev.token_estimate,
            retention_class,
            ev.created_at_epoch,
        ],
    )?;

    let id: i64 = conn.query_row(
        "SELECT id FROM captured_events
         WHERE host_id = ?1 AND session_id = ?2 AND event_id = ?3",
        params![host_id, ev.identity.session_id.0, ev.identity.event_id.0],
        |row| row.get(0),
    )?;
    Ok(id)
}

fn lookup_host_id(conn: &Connection, host: InstallHost) -> Result<i64> {
    conn.query_row(
        "SELECT id FROM hosts WHERE name = ?1",
        [host.as_db_value()],
        |row| row.get(0),
    )
    .with_context(|| {
        format!(
            "host '{}' is not seeded in the schema database; run admin reset-schema to initialize",
            host.as_db_value()
        )
    })
}

fn ensure_workspace(conn: &Connection, ws: &WorkspaceKey, now: i64) -> Result<i64> {
    let path_str = ws.root_path.to_string_lossy();
    conn.execute(
        "INSERT OR IGNORE INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?2)",
        params![path_str.as_ref(), now],
    )?;
    conn.query_row(
        "SELECT id FROM workspaces WHERE root_path = ?1",
        [path_str.as_ref()],
        |row| row.get::<_, i64>(0),
    )
    .map_err(Into::into)
}

fn ensure_project(conn: &Connection, workspace_id: i64, p: &ProjectKey, now: i64) -> Result<i64> {
    let path_str = p.project_path.to_string_lossy();
    conn.execute(
        "INSERT OR IGNORE INTO projects(workspace_id, project_path, project_key,
            created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![workspace_id, path_str.as_ref(), p.project_key, now],
    )?;
    conn.query_row(
        "SELECT id FROM projects
         WHERE workspace_id = ?1 AND project_path = ?2",
        params![workspace_id, path_str.as_ref()],
        |row| row.get::<_, i64>(0),
    )
    .map_err(Into::into)
}

/// Find or create the session row and bump its `last_seen_at_epoch`.
pub fn ensure_session(
    conn: &Connection,
    host_id: i64,
    workspace_id: i64,
    project_id: i64,
    session_id: &SessionId,
    now: i64,
) -> Result<i64> {
    conn.execute(
        "INSERT OR IGNORE INTO sessions(host_id, workspace_id, project_id, session_id,
            started_at_epoch, last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'active')",
        params![host_id, workspace_id, project_id, session_id.0, now],
    )?;
    conn.execute(
        "UPDATE sessions
         SET last_seen_at_epoch = MAX(last_seen_at_epoch, ?1)
         WHERE host_id = ?2 AND project_id = ?3 AND session_id = ?4",
        params![now, host_id, project_id, session_id.0],
    )?;
    conn.query_row(
        "SELECT id FROM sessions
         WHERE host_id = ?1 AND project_id = ?2 AND session_id = ?3",
        params![host_id, project_id, session_id.0],
        |row| row.get::<_, i64>(0),
    )
    .map_err(Into::into)
}

enum StorageKind {
    Empty,
    Inline,
    Spilled,
}

struct Storage {
    kind: StorageKind,
    content_text: Option<String>,
    content_blob_id: Option<i64>,
    content_hash: String,
}

/// Apply the capture storage policy.
/// - `None` content: empty hash, no inline text, no blob.
/// - ≤16 KiB: inline as `content_text`, blob unused.
/// - Over 16 KiB: spill the full text into `event_blobs`; `content_text`
///   keeps a compact prefix/suffix summary and retention becomes `raw_compact`.
fn resolve_storage(conn: &Connection, content: Option<&str>, now: i64) -> Result<Storage> {
    match content {
        None => Ok(Storage {
            kind: StorageKind::Empty,
            content_text: None,
            content_blob_id: None,
            content_hash: format!("{:016x}", crate::db::deterministic_hash(&[])),
        }),
        Some(text) if text.len() <= MAX_CONTENT_TEXT_BYTES => Ok(Storage {
            kind: StorageKind::Inline,
            content_text: Some(text.to_string()),
            content_blob_id: None,
            content_hash: format!("{:016x}", crate::db::deterministic_hash(text.as_bytes())),
        }),
        Some(text) => {
            let bytes = text.as_bytes();
            let hash = format!("{:016x}", crate::db::deterministic_hash(bytes));
            let blob_id = super::blob::insert_or_get_blob(conn, &hash, bytes, now)?;
            let summary = super::blob::summarize_oversize(text, 1024, 1024);
            Ok(Storage {
                kind: StorageKind::Spilled,
                content_text: Some(summary),
                content_blob_id: Some(blob_id),
                content_hash: hash,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::open_at as open_schema_at;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};
    use crate::identity::{
        CaptureIdentity, EventId, InstallHost, ProjectKey, SessionId, TurnId, WorkspaceKey,
    };
    use std::path::Path;

    fn unique_temp_path() -> std::path::PathBuf {
        unique_temp_db_path("capture")
    }

    fn make_identity(label: &str) -> CaptureIdentity {
        let workspace = WorkspaceKey::from_cwd_and_toplevel(Path::new("/tmp/repo-x"), None);
        let project = ProjectKey::from_workspace(workspace.clone(), Some("repo-x"));
        let session_id = SessionId(format!("session-{label}"));
        let turn_id = Some(TurnId(format!("turn-{label}")));
        let event_id = EventId::synthesize(turn_id.as_ref(), "PostToolUse", Some(label));
        CaptureIdentity {
            host: InstallHost::CodexCli,
            workspace,
            project,
            session_id,
            turn_id,
            event_id,
        }
    }

    fn make_event(label: &str, content: &str) -> NormalizedEvent {
        NormalizedEvent {
            identity: make_identity(label),
            event_type: "tool_result".into(),
            role: Some("tool".into()),
            tool_name: Some("Bash".into()),
            content_text: Some(content.into()),
            token_estimate: 0,
            retention_class: "raw_keep".into(),
            created_at_epoch: 1_700_000_000,
        }
    }

    #[test]
    fn insert_creates_fk_chain_and_returns_id() {
        let path = unique_temp_path();
        let conn = open_schema_at(&path).unwrap();
        let ev = make_event("a", "hello");
        let id = insert_captured_event(&conn, &ev).unwrap();
        assert!(id > 0);

        let host_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM hosts", [], |r| r.get(0))
            .unwrap();
        assert!(host_count >= 2, "seeded hosts must exist");
        let ws_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ws_count, 1);
        let proj_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .unwrap();
        assert_eq!(proj_count, 1);
        let sess_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sess_count, 1);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn duplicate_event_id_is_idempotent() {
        let path = unique_temp_path();
        let conn = open_schema_at(&path).unwrap();
        let ev = make_event("dup", "same");
        let id1 = insert_captured_event(&conn, &ev).unwrap();
        let id2 = insert_captured_event(&conn, &ev).unwrap();
        assert_eq!(id1, id2);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM captured_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn same_session_multiple_events_reuse_session_row() {
        let path = unique_temp_path();
        let conn = open_schema_at(&path).unwrap();

        let ws = WorkspaceKey::from_cwd_and_toplevel(Path::new("/tmp/r"), None);
        let project = ProjectKey::from_workspace(ws.clone(), Some("r"));
        let sid = SessionId("shared-session".into());
        let turn = Some(TurnId("turn1".into()));
        for n in 0..3 {
            let event_id = EventId::synthesize(turn.as_ref(), "PostToolUse", Some(&n.to_string()));
            let identity = CaptureIdentity {
                host: InstallHost::CodexCli,
                workspace: ws.clone(),
                project: project.clone(),
                session_id: sid.clone(),
                turn_id: turn.clone(),
                event_id,
            };
            let ev = NormalizedEvent {
                identity,
                event_type: "tool_result".into(),
                role: Some("tool".into()),
                tool_name: Some("Bash".into()),
                content_text: Some(format!("payload-{n}")),
                token_estimate: 0,
                retention_class: "raw_keep".into(),
                created_at_epoch: 1_700_000_000 + n,
            };
            insert_captured_event(&conn, &ev).unwrap();
        }
        let sess_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sess_count, 1);
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM captured_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(event_count, 3);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn oversize_content_spills_to_event_blobs_and_marks_raw_compact() {
        let path = unique_temp_path();
        let conn = open_schema_at(&path).unwrap();
        let big = "x".repeat(MAX_CONTENT_TEXT_BYTES + 5_000);
        let ev = make_event("big", &big);
        insert_captured_event(&conn, &ev).unwrap();

        let (stored_text, blob_id, retention): (String, Option<i64>, String) = conn
            .query_row(
                "SELECT content_text, content_blob_id, retention_class FROM captured_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        let blob_id = blob_id.expect("blob id must be set for spilled content");

        // content_text is the prefix/suffix summary, not the whole thing.
        assert!(
            stored_text.contains(&format!("{} bytes", big.len())),
            "summary must include size marker"
        );
        assert!(
            stored_text.len() < MAX_CONTENT_TEXT_BYTES,
            "summary stays small"
        );
        assert_eq!(retention, "raw_compact");

        // event_blobs has the full payload, plain encoding.
        let (encoding, original, stored): (String, i64, i64) = conn
            .query_row(
                "SELECT content_encoding, original_bytes, stored_bytes FROM event_blobs WHERE id = ?1",
                [blob_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(encoding, "plain");
        assert_eq!(original, big.len() as i64);
        assert_eq!(stored, original);
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn duplicate_oversize_content_dedupes_blob() {
        let path = unique_temp_path();
        let conn = open_schema_at(&path).unwrap();
        let big = "y".repeat(MAX_CONTENT_TEXT_BYTES + 1);
        let ev_a = make_event("dup-a", &big);
        let ev_b = make_event("dup-b", &big);
        insert_captured_event(&conn, &ev_a).unwrap();
        insert_captured_event(&conn, &ev_b).unwrap();
        let blob_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM event_blobs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(blob_count, 1, "same content_hash must dedupe to 1 blob");
        let event_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM captured_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(event_count, 2, "two distinct events still recorded");
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn lookup_host_id_returns_seeded_rows() {
        let path = unique_temp_path();
        let conn = open_schema_at(&path).unwrap();
        let claude = lookup_host_id(&conn, InstallHost::ClaudeCode).unwrap();
        let codex = lookup_host_id(&conn, InstallHost::CodexCli).unwrap();
        assert_ne!(claude, codex);
        cleanup_temp_db_files(&path);
    }
}
