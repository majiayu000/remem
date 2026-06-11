use anyhow::Result;
use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

use super::host::HostKind;
use super::invocation::ContextInvocation;

mod delta;

const DEFAULT_GATE_HOSTS: &str = "codex-cli,claude-code";
const DEFAULT_SUPPRESSED_SOURCES: &str = "compact";
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
    pub key: Option<String>,
    pub context_hash: Option<String>,
    pub output_mode: Option<&'static str>,
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
            crate::log::error(
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
                gate_decision(
                    output,
                    ContextGateAction::EmittedFull,
                    "first_or_forced",
                    &key,
                    &hash,
                    "full",
                )
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
                gate_decision(
                    output,
                    ContextGateAction::EmittedFull,
                    "first_or_forced",
                    &key,
                    &hash,
                    "full",
                )
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
                gate_decision(
                    output,
                    ContextGateAction::EmittedFull,
                    "restart_source",
                    &key,
                    &hash,
                    "full",
                )
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }
    let suppressed_source = source_is_suppressed(invocation.source.as_deref());
    if row.context_hash == hash {
        if suppressed_source || fallback_cooldown_allows_suppression(invocation, &row, now) {
            let reason = if suppressed_source {
                "suppressed_source"
            } else {
                "same_hash"
            };
            return match record_suppression(&conn, invocation, &key, now) {
                Ok(()) => {
                    log_gate("suppress", invocation, &key, reason, &hash);
                    gate_decision(
                        String::new(),
                        ContextGateAction::Suppressed,
                        reason,
                        &key,
                        &hash,
                        "suppressed",
                    )
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
                gate_decision(
                    output,
                    ContextGateAction::EmittedFull,
                    "fallback_cooldown_expired",
                    &key,
                    &hash,
                    "full",
                )
            }
            Err(error) => fail_open(output, "gate_write", error),
        };
    }

    if mode == ContextGateMode::Strict {
        return match record_suppression(&conn, invocation, &key, now) {
            Ok(()) => {
                log_gate("suppress", invocation, &key, "strict", &hash);
                gate_decision(
                    String::new(),
                    ContextGateAction::Suppressed,
                    "strict",
                    &key,
                    &hash,
                    "suppressed",
                )
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
            gate_decision(output, action, "changed_hash", &key, &hash, output_mode)
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
        key: None,
        context_hash: None,
        output_mode: None,
    }
}

fn gate_decision(
    output: String,
    action: ContextGateAction,
    reason: &'static str,
    key: &str,
    context_hash: &str,
    output_mode: &'static str,
) -> ContextGateDecision {
    ContextGateDecision {
        output,
        action,
        reason,
        key: Some(key.to_string()),
        context_hash: Some(context_hash.to_string()),
        output_mode: Some(output_mode),
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
        Some(value) if value == "clear"
    )
}

fn source_is_suppressed(source: Option<&str>) -> bool {
    let Some(source) = source.map(|value| value.trim().to_ascii_lowercase()) else {
        return false;
    };
    let suppressed_sources = std::env::var("REMEM_CONTEXT_SUPPRESS_SOURCES")
        .unwrap_or_else(|_| DEFAULT_SUPPRESSED_SOURCES.to_string());
    suppressed_sources
        .split(',')
        .map(|candidate| candidate.trim().to_ascii_lowercase())
        .any(|candidate| candidate == source)
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
    let mut skip_blank_after_source_note = false;
    for line in without_debug.lines() {
        let mut line = super::style::strip_ansi(line);
        strip_panel_right_border(&mut line);
        let line = line.as_str();
        if normalized.is_empty() && line.trim().is_empty() {
            continue;
        }
        if normalized.is_empty() {
            if matches!(line, "# remem context" | "# remem context delta")
                || matches!(line, "remem context" | "remem context delta")
                || line.starts_with("╭─ remem context")
                || line.starts_with("╭─ remem context delta")
                || line.starts_with("┌─ remem context")
                || line.starts_with("┌─ remem context delta")
            {
                normalized.push_str("# remem context\n");
                continue;
            }
            if let Some(prefix_end) = line.find("] context ") {
                normalized.push_str(&line[..prefix_end + "] context ".len()]);
                normalized.push_str("<timestamp>\n");
                continue;
            }
        }
        let line_for_match = line.trim_start();
        if skip_blank_after_source_note && line_for_match.trim().is_empty() {
            skip_blank_after_source_note = false;
            continue;
        }
        skip_blank_after_source_note = false;
        if is_context_source_note_line(line_for_match) {
            skip_blank_after_source_note = normalized.ends_with("\n\n");
            continue;
        }
        if is_visual_context_metadata_line(line_for_match)
            || line_for_match.starts_with("remem context source: ")
            || line_for_match.starts_with("REMEM_CONTEXT_SOURCE=")
        {
            continue;
        }
        if line_for_match.starts_with("╭─ Loaded") {
            normalized.push_str("## Loaded\n");
            continue;
        }
        if line_for_match.starts_with("┌─ Loaded") {
            normalized.push_str("## Loaded\n");
            continue;
        }
        if line_for_match == "Loaded" {
            normalized.push_str("## Loaded\n");
            continue;
        }
        if line_for_match.starts_with('╰') && line_for_match.ends_with('╯') {
            continue;
        }
        if let Some(row) = normalize_rail_row_for_hash(line_for_match) {
            normalized.push_str(&normalize_stats_footer_totals(row));
            normalized.push('\n');
            continue;
        }
        normalized.push_str(&normalize_stats_footer_totals(line));
        normalized.push('\n');
    }
    normalized
}

fn is_context_source_note_line(line: &str) -> bool {
    matches!(
        line,
        "Codex compacted the chat, so remem refreshed memory context."
            | "Context was reloaded after an explicit clear."
    )
}

fn strip_panel_right_border(line: &mut String) {
    if line.starts_with("│ ") && line.ends_with('│') {
        line.pop();
        while line.ends_with(' ') {
            line.pop();
        }
    }
}

fn is_visual_context_metadata_line(line: &str) -> bool {
    let row = normalize_rail_row_for_hash(line).unwrap_or(line);
    row.starts_with("- updated: ")
        || row.starts_with("- source: ")
        || row.starts_with("updated: ")
        || row.starts_with("source: ")
}

fn normalize_rail_row_for_hash(line: &str) -> Option<&str> {
    line.strip_prefix("├─ ")
        .or_else(|| line.strip_prefix("└─ "))
        .or_else(|| line.strip_prefix("│ "))
}

fn normalize_stats_footer_totals(line: &str) -> String {
    if let Some(normalized) = normalize_budget_footer_line(line) {
        return normalized;
    }

    const TOTAL_PREFIX: &str = " total=";
    const CHARS_TOKEN: &str = " chars/~";
    const TOKENS_SUFFIX: &str = " tokens";

    if !is_legacy_context_stats_footer_line(line) {
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

fn normalize_budget_footer_line(line: &str) -> Option<String> {
    let prefix = if line.starts_with("- Budget: ") {
        "- Budget: "
    } else if line.starts_with("│ Budget: ") {
        "│ Budget: "
    } else if line.starts_with("Budget: ") {
        "Budget: "
    } else {
        return None;
    };
    const CHARS_TOKEN: &str = " chars (~";
    const TOKENS_SUFFIX: &str = " tokens)";

    let char_value_start = prefix.len();
    let chars_offset = line.strip_prefix(prefix)?.find(CHARS_TOKEN)?;
    let char_value_end = char_value_start + chars_offset;
    if !line[char_value_start..char_value_end]
        .chars()
        .all(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    let token_value_start = char_value_end + CHARS_TOKEN.len();
    let tokens_offset = line[token_value_start..].find(TOKENS_SUFFIX)?;
    let token_value_end = token_value_start + tokens_offset;
    if !line[token_value_start..token_value_end]
        .chars()
        .all(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    let mut normalized = String::with_capacity(line.len());
    normalized.push_str(&line[..char_value_start]);
    normalized.push_str("<total>");
    normalized.push_str(CHARS_TOKEN);
    normalized.push_str("<tokens>");
    normalized.push_str(&line[token_value_end..]);
    Some(normalized)
}

fn is_legacy_context_stats_footer_line(line: &str) -> bool {
    line.contains(" context memories loaded. ")
        && line.contains(" host=")
        && line.contains(" branch=")
        && line.contains(" total=")
        && line.contains(" truncated=")
}

fn is_context_stats_footer(text: &str) -> bool {
    let text = super::style::strip_ansi(text);
    (text.starts_with("## Loaded\n")
        && text.contains("\n- Memories: ")
        && text.contains("\n- Preferences: ")
        && text.contains("\n- Budget: "))
        || (text.starts_with("╭─ Loaded")
            && text.contains("\n│ Memories: ")
            && text.contains("\n│ Preferences: ")
            && text.contains("\n│ Budget: "))
        || (text.starts_with("┌─ Loaded")
            && text.contains("\n├─ Memories: ")
            && text.contains("\n├─ Preferences: ")
            && text.contains("\n└─ Budget: "))
        || (text.starts_with("Loaded\n")
            && text.contains("\n├─ Memories: ")
            && text.contains("\n├─ Preferences: ")
            && text.contains("\n└─ Budget: "))
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
mod tests;
