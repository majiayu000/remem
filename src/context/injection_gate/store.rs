use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use super::data_version;
use super::ContextInvocation;

#[derive(Debug)]
pub(super) struct GateRow {
    pub(super) context_hash: String,
    pub(super) last_emitted_epoch: i64,
    pub(super) data_version: Option<String>,
}

pub(super) fn load_gate_row(
    conn: &rusqlite::Connection,
    host: &str,
    key: &str,
) -> Result<Option<GateRow>> {
    if data_version::context_injections_has_data_version(conn)? {
        return conn
            .query_row(
                "SELECT context_hash, last_emitted_epoch, data_version
                 FROM context_injections
                 WHERE host = ?1 AND injection_key = ?2",
                params![host, key],
                |row| {
                    Ok(GateRow {
                        context_hash: row.get(0)?,
                        last_emitted_epoch: row.get(1)?,
                        data_version: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into);
    }

    conn.query_row(
        "SELECT context_hash, last_emitted_epoch
         FROM context_injections
         WHERE host = ?1 AND injection_key = ?2",
        params![host, key],
        |row| {
            Ok(GateRow {
                context_hash: row.get(0)?,
                last_emitted_epoch: row.get(1)?,
                data_version: None,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn upsert_emit_row(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    key: &str,
    hash: &str,
    data_version: Option<&str>,
    output_mode: &str,
    output_chars: usize,
    now: i64,
) -> Result<()> {
    if data_version.is_some() && data_version::context_injections_has_data_version(conn)? {
        conn.execute(
            "INSERT INTO context_injections
             (host, project, injection_key, session_id, transcript_path, hook_source, context_hash,
              data_version, output_mode, output_chars, created_at_epoch, updated_at_epoch,
              last_emitted_epoch, emit_count, suppress_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?11, 1, 0)
             ON CONFLICT(host, injection_key) DO UPDATE SET
              project = excluded.project,
              session_id = excluded.session_id,
              transcript_path = excluded.transcript_path,
              hook_source = excluded.hook_source,
              context_hash = excluded.context_hash,
              data_version = excluded.data_version,
              output_mode = excluded.output_mode,
              output_chars = excluded.output_chars,
              updated_at_epoch = excluded.updated_at_epoch,
              last_emitted_epoch = excluded.last_emitted_epoch,
              emit_count = context_injections.emit_count + 1",
            params![
                invocation.host.as_env_value(),
                invocation.project,
                key,
                invocation.session_id,
                invocation.transcript_path,
                invocation.source,
                hash,
                data_version,
                output_mode,
                output_chars as i64,
                now,
            ],
        )?;
    } else {
        conn.execute(
            "INSERT INTO context_injections
             (host, project, injection_key, session_id, transcript_path, hook_source, context_hash,
              output_mode, output_chars, created_at_epoch, updated_at_epoch, last_emitted_epoch,
              emit_count, suppress_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10, 1, 0)
             ON CONFLICT(host, injection_key) DO UPDATE SET
              project = excluded.project,
              session_id = excluded.session_id,
              transcript_path = excluded.transcript_path,
              hook_source = excluded.hook_source,
              context_hash = excluded.context_hash,
              output_mode = excluded.output_mode,
              output_chars = excluded.output_chars,
              updated_at_epoch = excluded.updated_at_epoch,
              last_emitted_epoch = excluded.last_emitted_epoch,
              emit_count = context_injections.emit_count + 1",
            params![
                invocation.host.as_env_value(),
                invocation.project,
                key,
                invocation.session_id,
                invocation.transcript_path,
                invocation.source,
                hash,
                output_mode,
                output_chars as i64,
                now,
            ],
        )?;
    }
    Ok(())
}

pub(super) fn record_suppression(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    key: &str,
    data_version: Option<&str>,
    now: i64,
) -> Result<()> {
    if data_version.is_some() && data_version::context_injections_has_data_version(conn)? {
        conn.execute(
            "UPDATE context_injections
             SET output_mode = 'suppressed',
                 hook_source = ?3,
                 data_version = ?4,
                 updated_at_epoch = ?5,
                 suppress_count = suppress_count + 1
             WHERE host = ?1 AND injection_key = ?2",
            params![
                invocation.host.as_env_value(),
                key,
                invocation.source,
                data_version,
                now,
            ],
        )?;
    } else {
        conn.execute(
            "UPDATE context_injections
             SET output_mode = 'suppressed',
                 hook_source = ?3,
                 updated_at_epoch = ?4,
                 suppress_count = suppress_count + 1
             WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), key, invocation.source, now],
        )?;
    }
    Ok(())
}
