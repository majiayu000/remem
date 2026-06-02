use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

use crate::memory_candidate::ParsedMemoryCandidate;

const DEFAULT_CLAIM_LOOKBACK_SECS: i64 = 7_200;
const PREVIEW_CHARS: usize = 240;

pub(crate) struct ClaimWriteRequest<'a> {
    pub(crate) memory_id: i64,
    pub(crate) session_id: Option<&'a str>,
    pub(crate) host: Option<&'a str>,
    pub(crate) claim_source: &'a str,
}

struct ClaimMemory {
    id: i64,
    project: String,
    source_project: Option<String>,
    memory_type: String,
    topic_key: Option<String>,
    title: Option<String>,
    content: String,
    owner_scope: Option<String>,
    owner_key: Option<String>,
    branch: Option<String>,
}

struct MatchingClaim {
    id: i64,
    memory_id: i64,
    claim_source: String,
}

struct RecentClaim {
    id: i64,
    memory_id: i64,
    claim_source: String,
    topic_key: Option<String>,
    content_fingerprint: String,
    content: String,
}

pub(crate) fn claims_enabled(request_enabled: Option<bool>) -> bool {
    if request_enabled == Some(false) {
        return false;
    }
    match std::env::var("REMEM_MEMORY_CLAIMS") {
        Ok(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            !(normalized == "off" || normalized == "false" || normalized == "0")
        }
        Err(_) => true,
    }
}

pub(crate) fn insert_memory_claim(conn: &Connection, input: &ClaimWriteRequest<'_>) -> Result<i64> {
    let memory = load_claim_memory(conn, input.memory_id)?;
    let now = chrono::Utc::now().timestamp();
    let fingerprint = content_fingerprint(
        &memory.memory_type,
        memory.owner_scope.as_deref(),
        memory.owner_key.as_deref(),
        &memory.content,
    );
    let preview = claim_preview(&memory.content);
    let expires_at = now + claim_lookback_secs();
    conn.execute(
        "INSERT INTO memory_claims
         (memory_id, project, source_project, session_id, host, claim_source,
          memory_type, topic_key, title, content_fingerprint, content_preview,
          owner_scope, owner_key, branch, created_at_epoch, expires_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            memory.id,
            memory.project,
            memory.source_project,
            clean_claim_metadata(input.session_id),
            clean_claim_metadata(input.host),
            input.claim_source,
            memory.memory_type,
            memory.topic_key,
            memory.title,
            fingerprint,
            preview,
            memory.owner_scope,
            memory.owner_key,
            memory.branch,
            now,
            expires_at
        ],
    )
    .with_context(|| format!("insert memory claim for memory {}", input.memory_id))?;
    let claim_id = conn.last_insert_rowid();
    crate::log::info(
        "memory-claim",
        &format!(
            "saved id={} memory_id={} session={} source={}",
            claim_id,
            input.memory_id,
            input.session_id.unwrap_or("<none>"),
            input.claim_source
        ),
    );
    Ok(claim_id)
}

pub(crate) fn filter_summary_candidates_by_claims(
    conn: &Connection,
    session_id: &str,
    project: &str,
    candidates: &[ParsedMemoryCandidate],
) -> Vec<ParsedMemoryCandidate> {
    let session_id = session_id.trim();
    let summary_host = if session_id.is_empty() {
        None
    } else {
        match latest_session_host(conn, session_id, project) {
            Ok(host) => host,
            Err(err) => {
                crate::log::error(
                    "memory-claim",
                    &format!(
                        "claim host lookup failed session={} project={} error={}",
                        session_id, project, err
                    ),
                );
                None
            }
        }
    };

    let mut remaining = Vec::new();
    for candidate in candidates {
        match matching_claim(
            conn,
            session_id,
            summary_host.as_deref(),
            project,
            candidate,
        ) {
            Ok(Some(claim)) => {
                if let Err(err) =
                    record_noop_and_consume(conn, session_id, project, candidate, &claim)
                {
                    crate::log::error(
                        "memory-claim",
                        &format!(
                            "noop audit failed claim_id={} memory_id={} session={} type={} error={}",
                            claim.id, claim.memory_id, session_id, candidate.memory_type, err
                        ),
                    );
                    remaining.push(candidate.clone());
                }
            }
            Ok(None) => remaining.push(candidate.clone()),
            Err(err) => {
                crate::log::error(
                    "memory-claim",
                    &format!(
                        "claim lookup failed session={} project={} type={} error={}",
                        session_id, project, candidate.memory_type, err
                    ),
                );
                remaining.push(candidate.clone());
            }
        }
    }
    remaining
}

fn matching_claim(
    conn: &Connection,
    session_id: &str,
    host: Option<&str>,
    project: &str,
    candidate: &ParsedMemoryCandidate,
) -> Result<Option<MatchingClaim>> {
    if !session_id.trim().is_empty() {
        if let Some(claim) = exact_session_claim(conn, session_id, host, project, candidate)? {
            return Ok(Some(claim));
        }
    }
    recent_project_claim(conn, session_id, host, project, candidate)
}

fn latest_session_host(
    conn: &Connection,
    session_id: &str,
    project: &str,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT h.name
         FROM captured_events e
         JOIN hosts h ON h.id = e.host_id
         JOIN projects p ON p.id = e.project_id
         WHERE e.session_id = ?1
           AND p.project_path = ?2
         ORDER BY e.id DESC
         LIMIT 1",
        params![session_id, project],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn load_claim_memory(conn: &Connection, memory_id: i64) -> Result<ClaimMemory> {
    conn.query_row(
        "SELECT id, project, source_project, memory_type, topic_key, title, content,
                owner_scope, owner_key, branch
         FROM memories
         WHERE id = ?1",
        [memory_id],
        |row| {
            Ok(ClaimMemory {
                id: row.get(0)?,
                project: row.get(1)?,
                source_project: row.get(2)?,
                memory_type: row.get(3)?,
                topic_key: row.get(4)?,
                title: row.get(5)?,
                content: row.get(6)?,
                owner_scope: row.get(7)?,
                owner_key: row.get(8)?,
                branch: row.get(9)?,
            })
        },
    )
    .with_context(|| format!("load memory {memory_id} for claim"))
}

fn exact_session_claim(
    conn: &Connection,
    session_id: &str,
    host: Option<&str>,
    project: &str,
    candidate: &ParsedMemoryCandidate,
) -> Result<Option<MatchingClaim>> {
    let (owner_scope, owner_key) = candidate_owner(project, &candidate.scope);
    let fingerprint = content_fingerprint(
        &candidate.memory_type,
        Some(owner_scope),
        Some(owner_key),
        &candidate.text,
    );
    let now = chrono::Utc::now().timestamp();
    conn.query_row(
        "SELECT id, memory_id, claim_source
         FROM memory_claims
         WHERE project = ?1
           AND session_id = ?2
           AND memory_type = ?3
           AND content_fingerprint = ?4
           AND COALESCE(owner_scope, '') = ?5
           AND COALESCE(owner_key, '') = ?6
           AND consumed_at_epoch IS NULL
           AND (expires_at_epoch IS NULL OR expires_at_epoch > ?7)
           AND (?8 IS NULL OR host IS NULL OR host = ?8)
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT 1",
        params![
            project,
            session_id,
            candidate.memory_type,
            fingerprint,
            owner_scope,
            owner_key,
            now,
            host
        ],
        |row| {
            Ok(MatchingClaim {
                id: row.get(0)?,
                memory_id: row.get(1)?,
                claim_source: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn recent_project_claim(
    conn: &Connection,
    session_id: &str,
    host: Option<&str>,
    project: &str,
    candidate: &ParsedMemoryCandidate,
) -> Result<Option<MatchingClaim>> {
    let (owner_scope, owner_key) = candidate_owner(project, &candidate.scope);
    let candidate_fingerprint = content_fingerprint(
        &candidate.memory_type,
        Some(owner_scope),
        Some(owner_key),
        &candidate.text,
    );
    let candidate_normalized = normalize_claim_text(&candidate.text);
    let now = chrono::Utc::now().timestamp();
    let min_created_at = now - claim_lookback_secs();
    let mut stmt = conn.prepare(
        "SELECT c.id, c.memory_id, c.claim_source, c.topic_key,
                c.content_fingerprint, m.content
         FROM memory_claims c
         JOIN memories m ON m.id = c.memory_id
         WHERE c.project = ?1
           AND c.memory_type = ?2
           AND COALESCE(c.owner_scope, '') = ?3
           AND COALESCE(c.owner_key, '') = ?4
           AND c.consumed_at_epoch IS NULL
           AND c.created_at_epoch >= ?5
           AND (c.expires_at_epoch IS NULL OR c.expires_at_epoch > ?6)
           AND (?7 = '' OR c.session_id IS NULL OR c.session_id = ?7)
           AND (?8 IS NULL OR c.host IS NULL OR c.host = ?8)
         ORDER BY c.created_at_epoch DESC, c.id DESC
         LIMIT 25",
    )?;
    let rows = stmt.query_map(
        params![
            project,
            candidate.memory_type,
            owner_scope,
            owner_key,
            min_created_at,
            now,
            session_id.trim(),
            host
        ],
        |row| {
            Ok(RecentClaim {
                id: row.get(0)?,
                memory_id: row.get(1)?,
                claim_source: row.get(2)?,
                topic_key: row.get(3)?,
                content_fingerprint: row.get(4)?,
                content: row.get(5)?,
            })
        },
    )?;
    let claims = crate::db::query::collect_rows(rows)?;
    for claim in claims {
        if !topic_keys_compatible(claim.topic_key.as_deref(), Some(&candidate.topic_key)) {
            continue;
        }
        if claim.content_fingerprint == candidate_fingerprint
            || claim_text_similarity(&candidate_normalized, &normalize_claim_text(&claim.content))
                >= 0.86
        {
            return Ok(Some(MatchingClaim {
                id: claim.id,
                memory_id: claim.memory_id,
                claim_source: claim.claim_source,
            }));
        }
    }
    Ok(None)
}

fn record_noop_and_consume(
    conn: &Connection,
    session_id: &str,
    project: &str,
    candidate: &ParsedMemoryCandidate,
    claim: &MatchingClaim,
) -> Result<()> {
    with_claim_savepoint(conn, || {
        let now = chrono::Utc::now().timestamp();
        let reason = noop_reason_for_claim_source(&claim.claim_source);
        conn.execute(
            "INSERT INTO memory_candidate_noops
             (project, session_id, memory_claim_id, memory_id, memory_type,
              topic_key, candidate_text_preview, reason, created_at_epoch)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                project,
                session_id,
                claim.id,
                claim.memory_id,
                candidate.memory_type,
                candidate.topic_key,
                claim_preview(&candidate.text),
                reason,
                now
            ],
        )?;
        let updated = conn.execute(
            "UPDATE memory_claims
             SET consumed_at_epoch = ?1,
                 consumed_by_session_id = ?2,
                 consumed_reason = ?3
             WHERE id = ?4
               AND consumed_at_epoch IS NULL",
            params![now, session_id, reason, claim.id],
        )?;
        if updated == 0 {
            bail!("claim {} was already consumed", claim.id);
        }
        crate::log::info(
            "memory-claim",
            &format!(
                "consumed id={} memory_id={} session={} candidate_type={} reason={}",
                claim.id, claim.memory_id, session_id, candidate.memory_type, reason
            ),
        );
        Ok(())
    })
}

fn candidate_owner<'a>(project: &'a str, scope: &str) -> (&'static str, &'a str) {
    if scope == "global" {
        ("user", "user:default")
    } else {
        ("repo", project)
    }
}

fn content_fingerprint(
    memory_type: &str,
    owner_scope: Option<&str>,
    owner_key: Option<&str>,
    text: &str,
) -> String {
    let normalized = normalize_claim_text(text);
    let mut hasher = Sha256::new();
    hasher.update(memory_type.as_bytes());
    hasher.update(b"\0");
    hasher.update(owner_scope.unwrap_or("").as_bytes());
    hasher.update(b"\0");
    hasher.update(owner_key.unwrap_or("").as_bytes());
    hasher.update(b"\0");
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn claim_lookback_secs() -> i64 {
    std::env::var("REMEM_MEMORY_CLAIM_LOOKBACK_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CLAIM_LOOKBACK_SECS)
}

fn topic_keys_compatible(claim_topic_key: Option<&str>, candidate_topic_key: Option<&str>) -> bool {
    let claim_topic_key = claim_topic_key.and_then(non_empty);
    let candidate_topic_key = candidate_topic_key.and_then(non_empty);
    match (claim_topic_key, candidate_topic_key) {
        (Some(claim), Some(candidate)) => {
            claim == candidate || is_hash_like_topic_key(claim) || is_hash_like_topic_key(candidate)
        }
        _ => true,
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn is_hash_like_topic_key(topic_key: &str) -> bool {
    let lower = topic_key.to_ascii_lowercase();
    let mut parts = lower.rsplitn(2, ['-', '_']);
    let tail = parts.next().unwrap_or_default();
    let prefix = parts.next().unwrap_or_default();
    tail.len() >= 8
        && tail.chars().all(|ch| ch.is_ascii_hexdigit())
        && matches!(
            prefix,
            "decision"
                | "discovery"
                | "preference"
                | "bugfix"
                | "lesson"
                | "procedure"
                | "architecture"
        )
}

fn claim_text_similarity(left: &str, right: &str) -> f64 {
    if left == right {
        return 1.0;
    }
    let left_tokens = significant_tokens(left);
    let right_tokens = significant_tokens(right);
    if left_tokens.is_empty() || right_tokens.is_empty() {
        return 0.0;
    }
    let intersection = left_tokens.intersection(&right_tokens).count();
    let union = left_tokens.union(&right_tokens).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn significant_tokens(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    ch.is_ascii_punctuation() && ch != '/' && ch != '_' && ch != '-'
                })
                .to_string()
        })
        .filter(|token| token.chars().count() >= 3 || !token.is_ascii())
        .filter(|token| !is_claim_stop_token(token))
        .collect()
}

fn is_claim_stop_token(token: &str) -> bool {
    matches!(
        token,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "should"
            | "when"
            | "where"
            | "what"
            | "why"
            | "how"
            | "into"
            | "was"
            | "were"
    )
}

fn normalize_claim_text(text: &str) -> String {
    let text = strip_summary_context(text.trim());
    let mut parts = Vec::new();
    for line in text.lines() {
        let stripped = strip_list_marker(line.trim());
        let stripped = stripped.trim_matches(|ch: char| {
            ch.is_ascii_punctuation() && ch != '/' && ch != '_' && ch != '-' && ch != '#'
        });
        if !stripped.is_empty() {
            parts.push(stripped);
        }
    }
    let joined = parts.join(" ");
    let lowered: String = joined
        .chars()
        .map(|ch| {
            if ch.is_ascii() {
                ch.to_ascii_lowercase()
            } else {
                ch
            }
        })
        .collect();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_summary_context(text: &str) -> &str {
    if text.starts_with("[Context:") {
        if let Some((_, body)) = text.split_once("\n\n") {
            return body.trim();
        }
    }
    text
}

fn strip_list_marker(line: &str) -> &str {
    let line = line.trim_start_matches(['•', '-', '*', '·']).trim_start();
    let Some(first) = line.chars().next() else {
        return line;
    };
    if first.is_ascii_digit() {
        if let Some(pos) = line.find(". ") {
            return line[pos + 2..].trim_start();
        }
    }
    line
}

fn claim_preview(text: &str) -> String {
    crate::db::truncate_str(text.trim(), PREVIEW_CHARS).to_string()
}

fn clean_claim_metadata(value: Option<&str>) -> Option<&str> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn noop_reason_for_claim_source(source: &str) -> &'static str {
    match source {
        "api_save" => "covered_by_api_save",
        "cli_save" => "covered_by_cli_save",
        _ => "covered_by_manual_save",
    }
}

fn with_claim_savepoint<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT remem_claim_noop")?;
    match f() {
        Ok(value) => {
            conn.execute_batch("RELEASE SAVEPOINT remem_claim_noop")?;
            Ok(value)
        }
        Err(error) => {
            let rollback = conn.execute_batch(
                "ROLLBACK TO SAVEPOINT remem_claim_noop;
                 RELEASE SAVEPOINT remem_claim_noop;",
            );
            if let Err(rollback_error) = rollback {
                return Err(
                    error.context(format!("claim noop rollback also failed: {rollback_error}"))
                );
            }
            Err(error)
        }
    }
}
