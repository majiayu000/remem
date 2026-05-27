use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

use super::host::HostKind;
use super::invocation::ContextInvocation;

mod delta;

const DEFAULT_GATE_HOSTS: &str = "codex-cli";
const DEFAULT_FALLBACK_COOLDOWN_SECS: i64 = 900;
const DEFAULT_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextGateMode {
    Auto,
    Delta,
    Off,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContextGateAction {
    Bypassed,
    EmittedFull,
    EmittedDelta,
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
    last_emitted_epoch: i64,
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
    if !has_trusted_gate_identity(invocation) {
        crate::log::warn(
            "context-gate",
            &format!(
                "fail_open reason=missing_session_identity host={} project={} cwd={}",
                invocation.host.as_env_value(),
                invocation.project,
                invocation.cwd
            ),
        );
        return decision(
            output,
            ContextGateAction::FailOpen,
            "missing_session_identity",
        );
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
        return match upsert_emit_row(
            &conn,
            invocation,
            &key,
            &hash,
            "full",
            output.chars().count(),
            now,
        ) {
            Ok(()) => {
                log_gate("emit", invocation, &key, "full", &hash);
                decision(output, ContextGateAction::EmittedFull, "first_or_forced")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    };

    if invocation.force {
        return match upsert_emit_row(
            &conn,
            invocation,
            &key,
            &hash,
            "full",
            output.chars().count(),
            now,
        ) {
            Ok(()) => {
                log_gate("emit", invocation, &key, "full", &hash);
                decision(output, ContextGateAction::EmittedFull, "first_or_forced")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }
    if source_requires_fresh_emission(invocation.source.as_deref()) {
        return match upsert_emit_row(
            &conn,
            invocation,
            &key,
            &hash,
            "full",
            output.chars().count(),
            now,
        ) {
            Ok(()) => {
                log_gate("emit", invocation, &key, "restart_source", &hash);
                decision(output, ContextGateAction::EmittedFull, "restart_source")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }
    if row.context_hash == hash {
        if fallback_cooldown_allows_suppression(invocation, &row, now) {
            return match record_suppression(&conn, invocation, &key, now) {
                Ok(()) => {
                    log_gate("suppress", invocation, &key, "same_hash", &hash);
                    decision(String::new(), ContextGateAction::Suppressed, "same_hash")
                }
                Err(error) => fail_open(output, "gate_write", error),
            };
        }
        return match upsert_emit_row(
            &conn,
            invocation,
            &key,
            &hash,
            "full",
            output.chars().count(),
            now,
        ) {
            Ok(()) => {
                log_gate("emit", invocation, &key, "fallback_cooldown_expired", &hash);
                decision(
                    output,
                    ContextGateAction::EmittedFull,
                    "fallback_cooldown_expired",
                )
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }

    if mode == ContextGateMode::Strict {
        return match record_suppression(&conn, invocation, &key, now) {
            Ok(()) => {
                log_gate("suppress", invocation, &key, "strict", &hash);
                decision(String::new(), ContextGateAction::Suppressed, "strict")
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }

    let (output, action, output_mode) =
        if matches!(mode, ContextGateMode::Auto | ContextGateMode::Delta) {
            (
                delta::build_delta_output(&output),
                ContextGateAction::EmittedDelta,
                "delta",
            )
        } else {
            (output, ContextGateAction::EmittedFull, "full")
        };

    match upsert_emit_row(
        &conn,
        invocation,
        &key,
        &hash,
        output_mode,
        output.chars().count(),
        now,
    ) {
        Ok(()) => {
            log_gate("emit", invocation, &key, output_mode, &hash);
            decision(output, action, "changed_hash")
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
        "auto" => Some(ContextGateMode::Auto),
        "delta" => Some(ContextGateMode::Delta),
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

fn source_requires_fresh_emission(source: Option<&str>) -> bool {
    matches!(
        source.map(|value| value.trim().to_ascii_lowercase()),
        Some(value) if value == "clear" || value == "compact"
    )
}

fn has_trusted_gate_identity(invocation: &ContextInvocation) -> bool {
    invocation.session_id.is_some() || invocation.transcript_path.is_some()
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
    now.saturating_sub(row.last_emitted_epoch) <= cooldown
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
        "SELECT context_hash, last_emitted_epoch
         FROM context_injections
         WHERE host = ?1 AND injection_key = ?2",
        params![host, key],
        |row| {
            Ok(GateRow {
                context_hash: row.get(0)?,
                last_emitted_epoch: row.get(1)?,
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
    output_mode: &str,
    output_chars: usize,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO context_injections
         (host, project, injection_key, session_id, transcript_path, hook_source, context_hash, output_mode,
          output_chars, created_at_epoch, updated_at_epoch, last_emitted_epoch, emit_count,
          suppress_count)
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
    Ok(())
}

fn record_suppression(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    key: &str,
    now: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE context_injections
         SET output_mode = 'suppressed',
             hook_source = ?3,
             updated_at_epoch = ?4,
             suppress_count = suppress_count + 1
         WHERE host = ?1 AND injection_key = ?2",
        params![invocation.host.as_env_value(), key, invocation.source, now,],
    )?;
    Ok(())
}

fn injection_key(invocation: &ContextInvocation) -> String {
    if let Some(session_id) = invocation.session_id.as_deref() {
        return format!("session:{}:{}", invocation.project, session_id);
    }
    let cwd = fallback_cwd_key(&invocation.cwd);
    if let Some(transcript_path) = invocation.transcript_path.as_deref() {
        return format!(
            "fallback:{}:{}:{}",
            invocation.project,
            cwd,
            sha256_hex(transcript_path)
        );
    }
    format!("fallback:{}:{}", invocation.project, cwd)
}

fn fallback_cwd_key(cwd: &str) -> String {
    let path = Path::new(cwd);
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| normalize_path_lexically(path))
        .to_string_lossy()
        .to_string()
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() && !path.is_absolute() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

fn context_fingerprint(output: &str) -> String {
    sha256_hex(&normalize_context_for_hash(output))
}

fn normalize_context_for_hash(output: &str) -> String {
    const GENERATED_DEBUG_TRACE_MARKER: &str = "\n## Debug Trace\n- request host=";
    let without_debug = output
        .rfind(GENERATED_DEBUG_TRACE_MARKER)
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
        normalized.push_str(&normalize_stats_footer_totals(line));
        normalized.push('\n');
    }
    normalized
}

fn normalize_stats_footer_totals(line: &str) -> String {
    const TOTAL_PREFIX: &str = " total=";
    const CHARS_TOKEN: &str = " chars/~";
    const TOKENS_SUFFIX: &str = " tokens";

    if !is_context_stats_footer(line) {
        return line.to_string();
    }

    let Some(total_start) = line.find(TOTAL_PREFIX) else {
        return line.to_string();
    };
    let total_value_start = total_start + TOTAL_PREFIX.len();
    let Some(chars_offset) = line[total_value_start..].find(CHARS_TOKEN) else {
        return line.to_string();
    };
    let total_value_end = total_value_start + chars_offset;
    if !line[total_value_start..total_value_end]
        .chars()
        .all(|ch| ch.is_ascii_digit())
    {
        return line.to_string();
    }

    let token_value_start = total_value_end + CHARS_TOKEN.len();
    let Some(tokens_offset) = line[token_value_start..].find(TOKENS_SUFFIX) else {
        return line.to_string();
    };
    let token_value_end = token_value_start + tokens_offset;
    if !line[token_value_start..token_value_end]
        .chars()
        .all(|ch| ch.is_ascii_digit())
    {
        return line.to_string();
    }

    let mut normalized = String::with_capacity(line.len());
    normalized.push_str(&line[..total_value_start]);
    normalized.push_str("<total>");
    normalized.push_str(CHARS_TOKEN);
    normalized.push_str("<tokens>");
    normalized.push_str(&line[token_value_end..]);
    normalized
}

fn is_context_stats_footer(line: &str) -> bool {
    line.contains(" context memories loaded. ")
        && line.contains(" host=")
        && line.contains(" branch=")
        && line.contains(" total=")
        && line.contains(" truncated=")
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
            source: None,
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
    fn fingerprint_ignores_footer_totals_derived_from_header_width() {
        let a = "# [/tmp/remem] context 2026-05-25 9:00am\nBody\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=100 chars/~25 tokens limit=12000 truncated=no\n";
        let b = "# [/tmp/remem] context 2026-05-25 10:00am\nBody\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=101 chars/~26 tokens limit=12000 truncated=no\n";

        assert_eq!(context_fingerprint(a), context_fingerprint(b));
    }

    #[test]
    fn fingerprint_keeps_non_footer_total_text() {
        let a = "# [/tmp/remem] context now\nBody total=100 chars/~25 tokens\n";
        let b = "# [/tmp/remem] context now\nBody total=101 chars/~26 tokens\n";

        assert_ne!(context_fingerprint(a), context_fingerprint(b));
    }

    #[test]
    fn first_same_session_context_emits_and_second_suppresses() -> Result<()> {
        let conn = setup_conn();
        let invocation = invocation(Some("sess-1"));
        let key = injection_key(&invocation);
        let hash = context_fingerprint("# [/tmp/remem] context now\nBody\n");

        assert!(load_gate_row(&conn, invocation.host.as_env_value(), &key)?.is_none());
        upsert_emit_row(&conn, &invocation, &key, &hash, "full", 32, 100)?;
        let row = load_gate_row(&conn, invocation.host.as_env_value(), &key)?
            .ok_or_else(|| anyhow::anyhow!("missing context injection gate row"))?;
        assert_eq!(row.context_hash, hash);

        record_suppression(&conn, &invocation, &key, 101)?;
        let suppress_count: i64 = conn.query_row(
            "SELECT suppress_count FROM context_injections WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), key],
            |row| row.get(0),
        )?;
        assert_eq!(suppress_count, 1);
        Ok(())
    }

    #[test]
    fn fallback_injection_key_canonicalizes_equivalent_cwd() -> Result<()> {
        let cwd = std::env::current_dir()?;
        let mut direct = invocation(None);
        direct.transcript_path = None;
        direct.cwd = cwd.to_string_lossy().to_string();

        let mut dotted = direct.clone();
        dotted.cwd = cwd.join(".").to_string_lossy().to_string();

        assert_eq!(injection_key(&direct), injection_key(&dotted));
        Ok(())
    }

    #[test]
    fn transcript_fallback_injection_key_canonicalizes_equivalent_cwd() -> Result<()> {
        let cwd = std::env::current_dir()?;
        let mut direct = invocation(None);
        direct.cwd = cwd.to_string_lossy().to_string();
        direct.transcript_path = Some("/tmp/remem-transcript.jsonl".to_string());

        let mut dotted = direct.clone();
        dotted.cwd = cwd.join(".").to_string_lossy().to_string();

        assert_eq!(injection_key(&direct), injection_key(&dotted));
        Ok(())
    }

    #[test]
    fn cwd_only_fallback_identity_fails_open_without_suppressing() {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-cwd-only");
        let mut invocation = invocation(None);
        invocation.transcript_path = None;
        let output = "# [/tmp/remem] context now\nBody\n".to_string();

        let first = apply_context_gate(&invocation, output.clone());
        assert_eq!(first.action, ContextGateAction::FailOpen);
        assert_eq!(first.reason, "missing_session_identity");
        assert_eq!(first.output, output);

        let second = apply_context_gate(&invocation, output.clone());
        assert_eq!(second.action, ContextGateAction::FailOpen);
        assert_eq!(second.reason, "missing_session_identity");
        assert_eq!(second.output, output);
    }

    #[test]
    fn suppression_does_not_extend_fallback_cooldown() -> Result<()> {
        let conn = setup_conn();
        let invocation = invocation(None);
        let key = injection_key(&invocation);
        let hash = context_fingerprint("# [/tmp/remem] context now\nBody\n");

        upsert_emit_row(&conn, &invocation, &key, &hash, "full", 32, 100)?;
        record_suppression(&conn, &invocation, &key, 150)?;

        let (updated_at_epoch, last_emitted_epoch, suppress_count): (i64, i64, i64) = conn
            .query_row(
                "SELECT updated_at_epoch, last_emitted_epoch, suppress_count
                 FROM context_injections
                 WHERE host = ?1 AND injection_key = ?2",
                params![invocation.host.as_env_value(), key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;

        assert_eq!(updated_at_epoch, 150);
        assert_eq!(last_emitted_epoch, 100);
        assert_eq!(suppress_count, 1);
        Ok(())
    }

    #[test]
    fn restart_source_requires_fresh_emission() {
        assert!(source_requires_fresh_emission(Some("clear")));
        assert!(source_requires_fresh_emission(Some("Compact")));
        assert!(!source_requires_fresh_emission(Some("startup")));
        assert!(!source_requires_fresh_emission(None));
    }

    #[test]
    fn restart_source_reemits_same_hash_context() {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-restart");
        let mut invocation = invocation(Some("sess-restart"));
        let output = "# [/tmp/remem] context now\nBody\n".to_string();

        let first = apply_context_gate(&invocation, output.clone());
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let second = apply_context_gate(&invocation, output.clone());
        assert_eq!(second.action, ContextGateAction::Suppressed);

        invocation.source = Some("clear".to_string());
        let restart = apply_context_gate(&invocation, output.clone());
        assert_eq!(restart.action, ContextGateAction::EmittedFull);
        assert_eq!(restart.reason, "restart_source");
        assert_eq!(restart.output, output);
    }

    #[test]
    fn emit_and_suppress_persist_hook_source() -> Result<()> {
        let conn = setup_conn();
        let mut invocation = invocation(Some("sess-source"));
        invocation.source = Some("startup".to_string());
        let key = injection_key(&invocation);
        let hash = context_fingerprint("# [/tmp/remem] context now\nBody\n");

        upsert_emit_row(&conn, &invocation, &key, &hash, "full", 32, 100)?;
        invocation.source = Some("resume".to_string());
        record_suppression(&conn, &invocation, &key, 101)?;

        let hook_source: Option<String> = conn.query_row(
            "SELECT hook_source FROM context_injections WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), key],
            |row| row.get(0),
        )?;

        assert_eq!(hook_source.as_deref(), Some("resume"));
        Ok(())
    }

    #[test]
    fn fingerprint_keeps_user_debug_trace_heading() {
        let a = "# [/tmp/remem] context now\nBody\n## Debug Trace\nUser note A\n";
        let b = "# [/tmp/remem] context now\nBody\n## Debug Trace\nUser note B\n";

        assert_ne!(context_fingerprint(a), context_fingerprint(b));
    }

    #[test]
    fn fingerprint_ignores_generated_debug_trace() {
        let base = "# [/tmp/remem] context now\nBody\n";
        let a = format!(
            "{}\n## Debug Trace\n- request host=codex-cli project=/tmp/remem cwd=/tmp/remem branch=main session=sess-1\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=main total=100 chars/~25 tokens limit=12000 truncated=no\n",
            base
        );
        let b = format!(
            "{}\n## Debug Trace\n- request host=codex-cli project=/tmp/remem cwd=/tmp/remem branch=dev session=sess-2\n\n1 context memories loaded. 1 core (10 chars). 0 lessons (0 chars). 0 indexed (0 chars). 0 preferences (project:0 global:0, 0 chars). 0 sessions (0 chars). host=codex-cli branch=dev total=120 chars/~30 tokens limit=12000 truncated=no\n",
            base
        );

        assert_eq!(context_fingerprint(&a), context_fingerprint(&b));
    }
}
