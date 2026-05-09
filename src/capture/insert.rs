use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::types::NormalizedEvent;
use crate::identity::{InstallHost, ProjectKey, SessionId, WorkspaceKey};

/// Per SPEC-memory-system-v2.1-revisions §4 D1 — content_text up to 16 KiB
/// stays inline; bigger payloads spill into `event_blobs` (full prefix /
/// suffix + digest, gzip beyond 256 KiB). B.1 ships only the inline path
/// and truncates oversize content with `retention_class = "truncated"`; the
/// blob spill writer lands in B.1.x.
const MAX_CONTENT_TEXT_BYTES: usize = 16 * 1024;

/// Insert one normalized event into `captured_events`, creating any missing
/// hosts / workspaces / projects / sessions rows along the way. Idempotent
/// on `(host_id, session_id, event_id)`.
pub fn insert_captured_event(conn: &Connection, ev: &NormalizedEvent) -> Result<i64> {
    let host_id = lookup_host_id(conn, ev.identity.host)?;
    let workspace_id = ensure_workspace(conn, &ev.identity.workspace, ev.created_at_epoch)?;
    let project_id =
        ensure_project(conn, workspace_id, &ev.identity.project, ev.created_at_epoch)?;
    let session_row_id = ensure_session(
        conn,
        host_id,
        workspace_id,
        project_id,
        &ev.identity.session_id,
        ev.created_at_epoch,
    )?;

    let (content_text, retention_class) =
        compact_for_storage(ev.content_text.as_deref(), &ev.retention_class);
    let content_hash = compute_content_hash(content_text.as_deref());

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
            ?11, NULL, ?12,
            ?13, ?14,
            ?15, ?15
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
            content_text,
            content_hash,
            ev.token_estimate,
            retention_class,
            ev.created_at_epoch,
        ],
    )?;

    let id: i64 = conn.query_row(
        "SELECT id FROM captured_events
         WHERE host_id = ?1 AND session_id = ?2 AND event_id = ?3",
        params![
            host_id,
            ev.identity.session_id.0,
            ev.identity.event_id.0
        ],
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
            "host '{}' not seeded in v2 schema; run admin reset-v2 to initialize",
            host.as_db_value()
        )
    })
}

fn ensure_workspace(conn: &Connection, ws: &WorkspaceKey, now: i64) -> Result<i64> {
    let path_str = ws.root_path.to_string_lossy();
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM workspaces WHERE root_path = ?1",
            [path_str.as_ref()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO workspaces(root_path, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?2)",
        params![path_str.as_ref(), now],
    )?;
    Ok(conn.last_insert_rowid())
}

fn ensure_project(
    conn: &Connection,
    workspace_id: i64,
    p: &ProjectKey,
    now: i64,
) -> Result<i64> {
    let path_str = p.project_path.to_string_lossy();
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM projects
             WHERE workspace_id = ?1 AND project_path = ?2",
            params![workspace_id, path_str.as_ref()],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO projects(workspace_id, project_path, project_key,
            created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![workspace_id, path_str.as_ref(), p.project_key, now],
    )?;
    Ok(conn.last_insert_rowid())
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
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM sessions
             WHERE host_id = ?1 AND project_id = ?2 AND session_id = ?3",
            params![host_id, project_id, session_id.0],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    {
        conn.execute(
            "UPDATE sessions SET last_seen_at_epoch = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO sessions(host_id, workspace_id, project_id, session_id,
            started_at_epoch, last_seen_at_epoch, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, 'active')",
        params![host_id, workspace_id, project_id, session_id.0, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Apply the v2.1 §4 D1 storage policy. Returns `(content_text, retention)`.
/// B.1 minimal version: ≤16 KiB → keep, >16 KiB → truncate to 16 KiB on a
/// UTF-8 boundary + flip retention_class to "truncated". The 16-256 KiB and
/// >256 KiB blob cases land in B.1.x once the event_blobs writer ships.
fn compact_for_storage(
    content: Option<&str>,
    retention_class: &str,
) -> (Option<String>, String) {
    match content {
        None => (None, retention_class.to_string()),
        Some(text) if text.len() <= MAX_CONTENT_TEXT_BYTES => {
            (Some(text.to_string()), retention_class.to_string())
        }
        Some(text) => {
            let mut cut = MAX_CONTENT_TEXT_BYTES;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            (Some(text[..cut].to_string()), "truncated".to_string())
        }
    }
}

fn compute_content_hash(content: Option<&str>) -> String {
    let bytes = content.unwrap_or("").as_bytes();
    format!("{:016x}", crate::db::deterministic_hash(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{cleanup_temp_db_files, unique_temp_db_path};
    use crate::identity::{
        CaptureIdentity, EventId, InstallHost, ProjectKey, SessionId, TurnId, WorkspaceKey,
    };
    use crate::v2_db::open_v2_db_at;
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
        let conn = open_v2_db_at(&path).unwrap();
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
        let conn = open_v2_db_at(&path).unwrap();
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
        let conn = open_v2_db_at(&path).unwrap();

        let ws = WorkspaceKey::from_cwd_and_toplevel(Path::new("/tmp/r"), None);
        let project = ProjectKey::from_workspace(ws.clone(), Some("r"));
        let sid = SessionId("shared-session".into());
        let turn = Some(TurnId("turn1".into()));
        for n in 0..3 {
            let event_id =
                EventId::synthesize(turn.as_ref(), "PostToolUse", Some(&n.to_string()));
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
    fn oversize_content_is_truncated_and_marked() {
        let path = unique_temp_path();
        let conn = open_v2_db_at(&path).unwrap();
        let big = "x".repeat(MAX_CONTENT_TEXT_BYTES + 100);
        let ev = make_event("big", &big);
        insert_captured_event(&conn, &ev).unwrap();
        let (stored, retention): (String, String) = conn
            .query_row(
                "SELECT content_text, retention_class FROM captured_events",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored.len(), MAX_CONTENT_TEXT_BYTES);
        assert_eq!(retention, "truncated");
        cleanup_temp_db_files(&path);
    }

    #[test]
    fn truncate_respects_utf8_boundaries() {
        let mut s = "a".repeat(MAX_CONTENT_TEXT_BYTES - 1);
        s.push('中');
        s.push_str(&"b".repeat(50));
        let (out, retention) = compact_for_storage(Some(&s), "raw_keep");
        let out = out.unwrap();
        assert!(out.is_char_boundary(out.len()), "must end on a UTF-8 boundary");
        assert!(out.len() <= MAX_CONTENT_TEXT_BYTES);
        assert_eq!(retention, "truncated");
    }

    #[test]
    fn lookup_host_id_returns_seeded_rows() {
        let path = unique_temp_path();
        let conn = open_v2_db_at(&path).unwrap();
        let claude = lookup_host_id(&conn, InstallHost::ClaudeCode).unwrap();
        let codex = lookup_host_id(&conn, InstallHost::CodexCli).unwrap();
        assert_ne!(claude, codex);
        cleanup_temp_db_files(&path);
    }
}
