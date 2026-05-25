use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use super::host::HostKind;
use super::invocation::ContextInvocation;

const DEFAULT_GATE_HOSTS: &str = "codex-cli";
const DEFAULT_FALLBACK_COOLDOWN_SECS: i64 = 900;
const DEFAULT_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextGateMode {
    Auto,
    Off,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContextGateAction {
    Bypassed,
    EmittedFull,
    Suppressed,
    FailOpen,
}

#[derive(Debug, Clone)]
pub(super) struct ContextGateDecision {
    pub output: String,
    pub action: ContextGateAction,
    pub reason: &'static str,
}

#[derive(Debug)]
struct GateRow {
    context_hash: String,
    updated_at_epoch: i64,
}

pub(super) fn apply_context_gate(
    invocation: &ContextInvocation,
    output: String,
) -> ContextGateDecision {
    if output.is_empty() {
        return decision(output, ContextGateAction::Bypassed, "empty_output");
    }
    if !host_is_gated(invocation.host) {
        return decision(output, ContextGateAction::Bypassed, "host_not_gated");
    }

    let mode = resolve_gate_mode(invocation.gate_mode.as_deref());
    if mode == ContextGateMode::Off {
        return decision(output, ContextGateAction::Bypassed, "gate_off");
    }

    let hash = context_fingerprint(&output);
    let key = injection_key(invocation);
    let now = chrono::Utc::now().timestamp();
    let conn = match crate::db::open_db() {
        Ok(conn) => conn,
        Err(error) => {
            crate::log::warn(
                "context-gate",
                &format!("fail_open reason=open_db error={}", error),
            );
            return decision(output, ContextGateAction::FailOpen, "open_db");
        }
    };

    cleanup_old_rows(&conn, now);
    let row = match load_gate_row(&conn, invocation.host.as_env_value(), &key) {
        Ok(row) => row,
        Err(error) => {
            crate::log::warn(
                "context-gate",
                &format!("fail_open reason=gate_read error={}", error),
            );
            return decision(output, ContextGateAction::FailOpen, "gate_read");
        }
    };

    let Some(row) = row else {
        return match upsert_emit_row(&conn, invocation, &key, &hash, output.chars().count(), now) {
            Ok(()) => {
                log_gate("emit", invocation, &key, "full", &hash);
                decision(output, ContextGateAction::EmittedFull, "first_or_forced")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    };

    if invocation.force {
        return match upsert_emit_row(&conn, invocation, &key, &hash, output.chars().count(), now) {
            Ok(()) => {
                log_gate("emit", invocation, &key, "full", &hash);
                decision(output, ContextGateAction::EmittedFull, "first_or_forced")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }
    if row.context_hash == hash && fallback_cooldown_allows_suppression(invocation, &row, now) {
        return match record_suppression(&conn, invocation.host.as_env_value(), &key, now) {
            Ok(()) => {
                log_gate("suppress", invocation, &key, "same_hash", &hash);
                decision(String::new(), ContextGateAction::Suppressed, "same_hash")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }

    if mode == ContextGateMode::Strict {
        return match record_suppression(&conn, invocation.host.as_env_value(), &key, now) {
            Ok(()) => {
                log_gate("suppress", invocation, &key, "strict", &hash);
                decision(String::new(), ContextGateAction::Suppressed, "strict")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }

    match upsert_emit_row(&conn, invocation, &key, &hash, output.chars().count(), now) {
        Ok(()) => {
            log_gate("emit", invocation, &key, "changed_hash", &hash);
            decision(output, ContextGateAction::EmittedFull, "changed_hash")
        }
        Err(error) => fail_open(output, "gate_write", error),
    }
}

fn decision(
    output: String,
    action: ContextGateAction,
    reason: &'static str,
) -> ContextGateDecision {
    ContextGateDecision {
        output,
        action,
        reason,
    }
}

fn fail_open(output: String, reason: &'static str, error: anyhow::Error) -> ContextGateDecision {
    crate::log::warn(
        "context-gate",
        &format!("fail_open reason={} error={}", reason, error),
    );
    decision(output, ContextGateAction::FailOpen, reason)
}

fn resolve_gate_mode(cli_value: Option<&str>) -> ContextGateMode {
    let env_value = std::env::var("REMEM_CONTEXT_GATE").ok();
    cli_value
        .or(env_value.as_deref())
        .and_then(parse_gate_mode)
        .unwrap_or(ContextGateMode::Auto)
}

fn parse_gate_mode(value: &str) -> Option<ContextGateMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" | "delta" => Some(ContextGateMode::Auto),
        "off" | "false" | "0" => Some(ContextGateMode::Off),
        "strict" => Some(ContextGateMode::Strict),
        _ => None,
    }
}

fn host_is_gated(host: HostKind) -> bool {
    let hosts = std::env::var("REMEM_CONTEXT_GATE_HOSTS")
        .unwrap_or_else(|_| DEFAULT_GATE_HOSTS.to_string());
    hosts
        .split(',')
        .map(|host| host.trim())
        .any(|candidate| candidate == host.as_env_value())
}

fn fallback_cooldown_allows_suppression(
    invocation: &ContextInvocation,
    row: &GateRow,
    now: i64,
) -> bool {
    if invocation.session_id.is_some() {
        return true;
    }
    let cooldown = read_i64_env(
        "REMEM_CONTEXT_GATE_FALLBACK_COOLDOWN_SECS",
        DEFAULT_FALLBACK_COOLDOWN_SECS,
    );
    now.saturating_sub(row.updated_at_epoch) <= cooldown
}

fn read_i64_env(key: &str, default: i64) -> i64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(default)
}

fn cleanup_old_rows(conn: &rusqlite::Connection, now: i64) {
    let retention_days = read_i64_env("REMEM_CONTEXT_GATE_RETENTION_DAYS", DEFAULT_RETENTION_DAYS);
    let cutoff = now.saturating_sub(retention_days.saturating_mul(86_400));
    if let Err(error) = conn.execute(
        "DELETE FROM context_injections WHERE updated_at_epoch < ?1",
        [cutoff],
    ) {
        crate::log::warn(
            "context-gate",
            &format!("retention cleanup failed: {}", error),
        );
    }
}

fn load_gate_row(conn: &rusqlite::Connection, host: &str, key: &str) -> Result<Option<GateRow>> {
    conn.query_row(
        "SELECT context_hash, updated_at_epoch
         FROM context_injections
         WHERE host = ?1 AND injection_key = ?2",
        params![host, key],
        |row| {
            Ok(GateRow {
                context_hash: row.get(0)?,
                updated_at_epoch: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn upsert_emit_row(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    key: &str,
    hash: &str,
    output_chars: usize,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO context_injections
         (host, project, injection_key, session_id, transcript_path, context_hash, output_mode,
          output_chars, created_at_epoch, updated_at_epoch, last_emitted_epoch, emit_count,
          suppress_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'full', ?7, ?8, ?8, ?8, 1, 0)
         ON CONFLICT(host, injection_key) DO UPDATE SET
          project = excluded.project,
          session_id = excluded.session_id,
          transcript_path = excluded.transcript_path,
          context_hash = excluded.context_hash,
          output_mode = 'full',
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
            hash,
            output_chars as i64,
            now,
        ],
    )?;
    Ok(())
}

fn record_suppression(conn: &rusqlite::Connection, host: &str, key: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE context_injections
         SET output_mode = 'suppressed',
             updated_at_epoch = ?3,
             suppress_count = suppress_count + 1
         WHERE host = ?1 AND injection_key = ?2",
        params![host, key, now],
    )?;
    Ok(())
}

fn injection_key(invocation: &ContextInvocation) -> String {
    if let Some(session_id) = invocation.session_id.as_deref() {
        return format!("session:{}:{}", invocation.project, session_id);
    }
    if let Some(transcript_path) = invocation.transcript_path.as_deref() {
        return format!(
            "fallback:{}:{}:{}",
            invocation.project,
            invocation.cwd,
            sha256_hex(transcript_path)
        );
    }
    format!("fallback:{}:{}", invocation.project, invocation.cwd)
}

fn context_fingerprint(output: &str) -> String {
    sha256_hex(&normalize_context_for_hash(output))
}

fn normalize_context_for_hash(output: &str) -> String {
    let without_debug = output
        .find("\n## Debug Trace\n")
        .map(|idx| &output[..idx])
        .unwrap_or(output);
    let mut normalized = String::new();
    for (idx, line) in without_debug.lines().enumerate() {
        if idx == 0 {
            if let Some(prefix_end) = line.find("] context ") {
                normalized.push_str(&line[..prefix_end + "] context ".len()]);
                normalized.push_str("<timestamp>\n");
                continue;
            }
        }
        normalized.push_str(line);
        normalized.push('\n');
    }
    normalized
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{:02x}", byte));
    }
    out
}

fn log_gate(action: &str, invocation: &ContextInvocation, key: &str, reason: &str, hash: &str) {
    crate::log::info(
        "context-gate",
        &format!(
            "{} host={} key={} reason={} hash={} project={}",
            action,
            invocation.host.as_env_value(),
            key,
            reason,
            hash,
            invocation.project
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_conn() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../migrations/v016_context_injection_gate.sql"
        ))
        .unwrap();
        conn
    }

    fn invocation(session_id: Option<&str>) -> ContextInvocation {
        ContextInvocation {
            cwd: "/tmp/remem".to_string(),
            project: "/tmp/remem".to_string(),
            session_id: session_id.map(str::to_string),
            transcript_path: Some("/tmp/remem.jsonl".to_string()),
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: None,
        }
    }

    #[test]
    fn fingerprint_ignores_header_timestamp() {
        let a = "# [/tmp/remem] context 2026-05-25 1:00pm\nBody\n";
        let b = "# [/tmp/remem] context 2026-05-25 2:00pm\nBody\n";

        assert_eq!(context_fingerprint(a), context_fingerprint(b));
    }

    #[test]
    fn first_same_session_context_emits_and_second_suppresses() {
        let conn = setup_conn();
        let invocation = invocation(Some("sess-1"));
        let key = injection_key(&invocation);
        let hash = context_fingerprint("# [/tmp/remem] context now\nBody\n");

        assert!(load_gate_row(&conn, invocation.host.as_env_value(), &key)
            .unwrap()
            .is_none());
        upsert_emit_row(&conn, &invocation, &key, &hash, 32, 100).unwrap();
        let row = load_gate_row(&conn, invocation.host.as_env_value(), &key)
            .unwrap()
            .unwrap();
        assert_eq!(row.context_hash, hash);

        record_suppression(&conn, invocation.host.as_env_value(), &key, 101).unwrap();
        let suppress_count: i64 = conn
            .query_row(
                "SELECT suppress_count FROM context_injections WHERE host = ?1 AND injection_key = ?2",
                params![invocation.host.as_env_value(), key],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(suppress_count, 1);
    }
}
