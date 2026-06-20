use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::claims::{DEFAULT_OWNER_KEY, DEFAULT_OWNER_SCOPE, DEFAULT_USER_KEY};
mod types;
pub use types::{
    ActivityRef, DroppedSource, SummaryClaimSource, SummaryEditRequest, SummaryMemorySource,
    SummaryRequest, SummarySources, UserContextSummary,
};
use types::{ClaimCandidate, SourceBundle, SummaryRow};

const SUMMARY_SCOPE: &str = "project";
const SUMMARY_COMPILER_MODEL: &str = "deterministic-profile-v1";
const SUMMARY_EDIT_MODEL: &str = "manual-edit";
const MAX_CLAIMS: usize = 8;
const MAX_MEMORIES: usize = 6;
const MAX_ACTIVITIES: usize = 6;

pub fn load_active_summary(
    conn: &Connection,
    req: &SummaryRequest<'_>,
) -> Result<Option<UserContextSummary>> {
    let Some(summary) = load_active_summary_unfiltered(conn, req)? else {
        return Ok(None);
    };
    if summary_sources_are_visible(conn, &summary)? {
        Ok(Some(summary))
    } else {
        Ok(None)
    }
}

fn load_active_summary_unfiltered(
    conn: &Connection,
    req: &SummaryRequest<'_>,
) -> Result<Option<UserContextSummary>> {
    let (owner_scope, owner_key, project) = normalize_summary_request(req)?;
    let row = conn
        .query_row(
            "SELECT id, user_key, owner_scope, owner_key, scope, scope_key, summary_text,
                    source_claim_ids_json, source_memory_ids_json, source_activity_refs_json,
                    status, model, version, created_at_epoch, updated_at_epoch
             FROM user_context_summaries
             WHERE owner_scope = ?1 AND owner_key = ?2
               AND scope = ?3 AND scope_key = ?4 AND status = 'active'
             ORDER BY version DESC, id DESC
             LIMIT 1",
            params![owner_scope, owner_key, SUMMARY_SCOPE, project],
            map_summary_row,
        )
        .optional()?;
    row.map(summary_from_row).transpose()
}

pub fn refresh_summary(conn: &Connection, req: &SummaryRequest<'_>) -> Result<UserContextSummary> {
    refresh_summary_with_generator(conn, req, |project, sources| {
        Ok(compile_summary_text(project, sources))
    })
}

fn refresh_summary_with_generator<F>(
    conn: &Connection,
    req: &SummaryRequest<'_>,
    generator: F,
) -> Result<UserContextSummary>
where
    F: FnOnce(&str, &SourceBundle) -> Result<String>,
{
    let (owner_scope, owner_key, project) = normalize_summary_request(req)?;
    let sources = collect_sources(conn, &owner_scope, &owner_key, &project)
        .context("load profile summary sources")?;
    let summary_text = generator(&project, &sources).context("generate profile summary")?;
    insert_active_summary(
        conn,
        &owner_scope,
        &owner_key,
        &project,
        &summary_text,
        &sources
            .claims
            .iter()
            .map(|source| source.id)
            .collect::<Vec<_>>(),
        &sources
            .memories
            .iter()
            .map(|source| source.id)
            .collect::<Vec<_>>(),
        &sources.activity_refs,
        SUMMARY_COMPILER_MODEL,
    )
}

pub fn edit_summary(conn: &Connection, req: &SummaryEditRequest<'_>) -> Result<UserContextSummary> {
    let text = req.text.trim();
    if text.is_empty() {
        bail!("summary text cannot be empty");
    }
    let summary_req = SummaryRequest {
        owner_scope: req.owner_scope,
        owner_key: req.owner_key,
        project: req.project,
    };
    let existing = load_active_summary(conn, &summary_req)?;
    let (owner_scope, owner_key, project) = normalize_summary_request(&summary_req)?;
    let source_claim_ids = existing
        .as_ref()
        .map(|summary| summary.source_claim_ids.clone())
        .unwrap_or_default();
    let source_memory_ids = existing
        .as_ref()
        .map(|summary| summary.source_memory_ids.clone())
        .unwrap_or_default();
    let source_activity_refs = existing
        .as_ref()
        .map(|summary| summary.source_activity_refs.clone())
        .unwrap_or_default();
    insert_active_summary(
        conn,
        &owner_scope,
        &owner_key,
        &project,
        text,
        &source_claim_ids,
        &source_memory_ids,
        &source_activity_refs,
        SUMMARY_EDIT_MODEL,
    )
}

pub fn load_summary_sources(
    conn: &Connection,
    req: &SummaryRequest<'_>,
    include_excluded: bool,
) -> Result<SummarySources> {
    let summary = if include_excluded {
        load_active_summary_unfiltered(conn, req)?
    } else {
        load_active_summary(conn, req)?
    };
    let (owner_scope, owner_key, project) = normalize_summary_request(req)?;
    let mut sources = collect_sources(conn, &owner_scope, &owner_key, &project)?;
    if !include_excluded {
        sources.dropped_claims.clear();
    }
    if let Some(summary_ref) = &summary {
        let included_claims = load_claim_sources_by_ids(conn, &summary_ref.source_claim_ids)?;
        let included_memories = load_memory_sources_by_ids(conn, &summary_ref.source_memory_ids)?;
        let included_activity_refs = summary_ref.source_activity_refs.clone();
        sources
            .dropped_claims
            .retain(|source| !summary_ref.source_claim_ids.contains(&source.id));
        return Ok(SummarySources {
            summary,
            included_claims,
            included_memories,
            included_activity_refs,
            dropped_claims: sources.dropped_claims,
        });
    }
    Ok(SummarySources {
        summary,
        included_claims: sources.claims,
        included_memories: sources.memories,
        included_activity_refs: sources.activity_refs,
        dropped_claims: sources.dropped_claims,
    })
}

fn insert_active_summary(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    project: &str,
    summary_text: &str,
    source_claim_ids: &[i64],
    source_memory_ids: &[i64],
    source_activity_refs: &[ActivityRef],
    model: &str,
) -> Result<UserContextSummary> {
    let source_claim_ids_json = encode_ids(source_claim_ids)?;
    let source_memory_ids_json = encode_ids(source_memory_ids)?;
    let source_activity_refs_json = encode_activity_refs(source_activity_refs)?;
    let now = chrono::Utc::now().timestamp();
    let tx = conn.unchecked_transaction()?;
    let previous = load_active_summary_unfiltered(
        &tx,
        &SummaryRequest {
            owner_scope: Some(owner_scope),
            owner_key: Some(owner_key),
            project,
        },
    )?;
    let version = previous.as_ref().map_or(1, |summary| summary.version + 1);
    tx.execute(
        "UPDATE user_context_summaries
         SET status = 'superseded', updated_at_epoch = ?1
         WHERE owner_scope = ?2 AND owner_key = ?3
           AND scope = ?4 AND scope_key = ?5 AND status = 'active'",
        params![now, owner_scope, owner_key, SUMMARY_SCOPE, project],
    )?;
    tx.execute(
        "INSERT INTO user_context_summaries
         (user_key, owner_scope, owner_key, scope, scope_key, summary_text,
          source_claim_ids_json, source_memory_ids_json, source_activity_refs_json,
          status, model, version, created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?11, ?12, ?12)",
        params![
            DEFAULT_USER_KEY,
            owner_scope,
            owner_key,
            SUMMARY_SCOPE,
            project,
            summary_text,
            source_claim_ids_json,
            source_memory_ids_json,
            source_activity_refs_json,
            model,
            version,
            now,
        ],
    )?;
    let id = tx.last_insert_rowid();
    let summary = load_summary_by_id(&tx, id)?;
    tx.commit()?;
    Ok(summary)
}

fn summary_sources_are_visible(conn: &Connection, summary: &UserContextSummary) -> Result<bool> {
    if summary_is_policy_suppressed(conn, summary)? {
        return Ok(false);
    }
    for claim_id in &summary.source_claim_ids {
        if !summary_claim_source_is_visible(conn, *claim_id)? {
            return Ok(false);
        }
    }
    for memory_id in &summary.source_memory_ids {
        if !summary_memory_source_is_visible(conn, *memory_id)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn summary_is_policy_suppressed(conn: &Connection, summary: &UserContextSummary) -> Result<bool> {
    let summary_id = summary.id.to_string();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM memory_suppressions
         WHERE status = 'active'
           AND target_kind = 'summary'
           AND (
                target_id = ?1
             OR (target_value IS NOT NULL
                 AND (
                    target_value = ?2
                  OR instr(lower(?3), lower(target_value)) > 0
                 ))
           )",
        params![summary.id, summary_id, summary.summary_text],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn summary_claim_source_is_visible(conn: &Connection, claim_id: i64) -> Result<bool> {
    let now = chrono::Utc::now().timestamp();
    let sql = format!(
        "SELECT COUNT(*)
         FROM user_context_claims
         WHERE id = ?1
           AND status = 'active'
           AND sensitivity NOT IN ('personal', 'sensitive', 'restricted')
           AND (valid_from_epoch IS NULL OR valid_from_epoch <= ?2)
           AND (valid_to_epoch IS NULL OR valid_to_epoch > ?3)
           AND {}",
        crate::memory::suppression::user_claim_policy_filter_sql("user_context_claims"),
    );
    let count: i64 = conn.query_row(&sql, params![claim_id, now, now], |row| row.get(0))?;
    Ok(count > 0)
}

fn summary_memory_source_is_visible(conn: &Connection, memory_id: i64) -> Result<bool> {
    let sql = format!(
        "SELECT COUNT(*)
         FROM memories
         WHERE id = ?1
           AND {}
           AND {}
           AND {}",
        crate::memory::memory_current_filter_sql("status", "expires_at_epoch", false),
        crate::memory::memory_not_superseded_filter_sql("memories"),
        crate::memory::suppression::memory_policy_filter_sql("memories"),
    );
    let count: i64 = conn.query_row(&sql, [memory_id], |row| row.get(0))?;
    Ok(count > 0)
}

fn load_summary_by_id(conn: &Connection, id: i64) -> Result<UserContextSummary> {
    let row = conn.query_row(
        "SELECT id, user_key, owner_scope, owner_key, scope, scope_key, summary_text,
                source_claim_ids_json, source_memory_ids_json, source_activity_refs_json,
                status, model, version, created_at_epoch, updated_at_epoch
         FROM user_context_summaries
         WHERE id = ?1",
        [id],
        map_summary_row,
    )?;
    summary_from_row(row)
}

fn collect_sources(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    project: &str,
) -> Result<SourceBundle> {
    let (claims, dropped_claims) = load_claim_sources(conn, owner_scope, owner_key, project)?;
    let memories = load_memory_sources(conn, project)?;
    let activity_refs = load_activity_refs(conn, project)?;
    Ok(SourceBundle {
        claims,
        memories,
        activity_refs,
        dropped_claims,
    })
}

fn load_claim_sources(
    conn: &Connection,
    owner_scope: &str,
    owner_key: &str,
    project: &str,
) -> Result<(Vec<SummaryClaimSource>, Vec<DroppedSource>)> {
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn.prepare(&format!(
        "SELECT id, claim_type, claim_key, claim_text, owner_scope, owner_key,
                sensitivity, status, valid_from_epoch, valid_to_epoch
         FROM user_context_claims
         WHERE ((owner_scope = ?1 AND owner_key = ?2)
            OR (owner_scope = 'repo' AND owner_key = ?3))
           AND {policy_filter}
         ORDER BY updated_at_epoch DESC, id DESC
         LIMIT 50",
        policy_filter =
            crate::memory::suppression::user_claim_policy_filter_sql("user_context_claims"),
    ))?;
    let rows = stmt.query_map(params![owner_scope, owner_key, project], |row| {
        Ok(ClaimCandidate {
            id: row.get(0)?,
            claim_type: row.get(1)?,
            claim_key: row.get(2)?,
            claim_text: row.get(3)?,
            owner_scope: row.get(4)?,
            owner_key: row.get(5)?,
            sensitivity: row.get(6)?,
            status: row.get(7)?,
            valid_from_epoch: row.get(8)?,
            valid_to_epoch: row.get(9)?,
        })
    })?;
    let candidates = crate::db::query::collect_rows(rows)?;
    let mut included = Vec::new();
    let mut dropped = Vec::new();
    for candidate in candidates {
        if let Some(reason) = drop_reason_for_claim(&candidate, now) {
            dropped.push(DroppedSource {
                kind: "user_claim".to_string(),
                id: candidate.id,
                reason,
            });
            continue;
        }
        if included.len() < MAX_CLAIMS {
            included.push(SummaryClaimSource {
                id: candidate.id,
                claim_type: candidate.claim_type,
                claim_key: candidate.claim_key,
                claim_text: candidate.claim_text,
                owner_scope: candidate.owner_scope,
                owner_key: candidate.owner_key,
                sensitivity: candidate.sensitivity,
                status: candidate.status,
            });
        }
    }
    Ok((included, dropped))
}

fn load_claim_sources_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<SummaryClaimSource>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, claim_type, claim_key, claim_text, owner_scope, owner_key,
                sensitivity, status
         FROM user_context_claims
         WHERE id = ?1
           AND {policy_filter}",
        policy_filter =
            crate::memory::suppression::user_claim_policy_filter_sql("user_context_claims"),
    ))?;
    let mut sources = Vec::new();
    for id in ids {
        if let Some(source) = stmt
            .query_row(params![id], |row| {
                Ok(SummaryClaimSource {
                    id: row.get(0)?,
                    claim_type: row.get(1)?,
                    claim_key: row.get(2)?,
                    claim_text: row.get(3)?,
                    owner_scope: row.get(4)?,
                    owner_key: row.get(5)?,
                    sensitivity: row.get(6)?,
                    status: row.get(7)?,
                })
            })
            .optional()?
        {
            sources.push(source);
        }
    }
    Ok(sources)
}

fn load_memory_sources(conn: &Connection, project: &str) -> Result<Vec<SummaryMemorySource>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, title, content, memory_type, owner_scope, owner_key, status
         FROM memories
         WHERE status = 'active'
           AND (expires_at_epoch IS NULL OR expires_at_epoch > CAST(strftime('%s', 'now') AS INTEGER))
           AND {policy_filter}
           AND (
                (owner_scope = 'repo' AND owner_key = ?1)
             OR (owner_scope = 'repo' AND target_project = ?1)
             OR (owner_scope IS NULL AND project = ?1)
             OR (owner_scope = 'user' AND owner_key = 'user:default' AND memory_type = 'preference')
           )
         ORDER BY updated_at_epoch DESC, id DESC
         LIMIT ?2",
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("memories"),
    ))?;
    let rows = stmt.query_map(params![project, MAX_MEMORIES as i64], map_memory_source)?;
    crate::db::query::collect_rows(rows)
}

fn load_memory_sources_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<SummaryMemorySource>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id, title, content, memory_type, owner_scope, owner_key, status
         FROM memories
         WHERE id = ?1
           AND {policy_filter}",
        policy_filter = crate::memory::suppression::memory_policy_filter_sql("memories"),
    ))?;
    let mut sources = Vec::new();
    for id in ids {
        if let Some(source) = stmt.query_row(params![id], map_memory_source).optional()? {
            sources.push(source);
        }
    }
    Ok(sources)
}

fn map_memory_source(row: &rusqlite::Row<'_>) -> rusqlite::Result<SummaryMemorySource> {
    let content: String = row.get(2)?;
    Ok(SummaryMemorySource {
        id: row.get(0)?,
        title: row.get(1)?,
        preview: compact_line(&content, 160),
        memory_type: row.get(3)?,
        owner_scope: row.get(4)?,
        owner_key: row.get(5)?,
        status: row.get(6)?,
    })
}

fn load_activity_refs(conn: &Connection, project: &str) -> Result<Vec<ActivityRef>> {
    let mut refs = Vec::new();
    let mut workstreams = crate::workstream::query_active_workstreams(conn, project)?;
    workstreams.truncate(3);
    refs.extend(workstreams.into_iter().map(|workstream| ActivityRef {
        kind: "workstream".to_string(),
        id: workstream.id,
        label: compact_line(&workstream.title, 120),
    }));

    let remaining = MAX_ACTIVITIES.saturating_sub(refs.len());
    if remaining == 0 {
        return Ok(refs);
    }
    let mut stmt = conn.prepare(
        "SELECT id, COALESCE(request, completed, learned, decisions, next_steps, preferences, memory_session_id)
         FROM session_summaries
         WHERE ((owner_scope = 'repo' AND owner_key = ?1)
             OR (owner_scope = 'repo' AND target_project = ?1)
             OR (owner_scope IS NULL AND project = ?1))
         ORDER BY created_at_epoch DESC, id DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![project, remaining as i64], |row| {
        let label: String = row.get(1)?;
        Ok(ActivityRef {
            kind: "session_summary".to_string(),
            id: row.get(0)?,
            label: compact_line(&label, 120),
        })
    })?;
    refs.extend(crate::db::query::collect_rows(rows)?);
    Ok(refs)
}

fn compile_summary_text(project: &str, sources: &SourceBundle) -> String {
    let mut lines = Vec::new();
    for claim in &sources.claims {
        lines.push(format!(
            "- {} [claim:{}]",
            compact_line(&claim.claim_text, 180),
            claim.id
        ));
    }
    for memory in &sources.memories {
        lines.push(format!(
            "- {}: {} [memory:{}]",
            compact_line(&memory.title, 80),
            memory.preview,
            memory.id
        ));
    }
    for activity in &sources.activity_refs {
        lines.push(format!(
            "- {}: {} [{}:{}]",
            activity.kind,
            compact_line(&activity.label, 120),
            activity.kind,
            activity.id
        ));
    }
    if lines.is_empty() {
        return String::new();
    }
    format!("Profile summary for {project}\n{}", lines.join("\n"))
}

fn drop_reason_for_claim(candidate: &ClaimCandidate, now: i64) -> Option<String> {
    if candidate.status != "active" {
        return Some(format!("status:{}", candidate.status));
    }
    if matches!(
        candidate.sensitivity.as_str(),
        "personal" | "sensitive" | "restricted"
    ) {
        return Some(format!("sensitivity:{}", candidate.sensitivity));
    }
    if candidate
        .valid_from_epoch
        .is_some_and(|valid_from| valid_from > now)
    {
        return Some("not_yet_valid".to_string());
    }
    if candidate
        .valid_to_epoch
        .is_some_and(|valid_to| valid_to <= now)
    {
        return Some("expired".to_string());
    }
    None
}

fn normalize_summary_request(req: &SummaryRequest<'_>) -> Result<(String, String, String)> {
    let project = req.project.trim();
    if project.is_empty() {
        bail!("summary project cannot be empty");
    }
    let owner_scope = req
        .owner_scope
        .map(str::trim)
        .unwrap_or(DEFAULT_OWNER_SCOPE);
    validate_owner_scope(owner_scope)?;
    let owner_key = req.owner_key.map(str::trim).filter(|key| !key.is_empty());
    let owner_key = match (owner_scope, owner_key) {
        ("user", None) => DEFAULT_OWNER_KEY,
        (_, Some(owner_key)) => owner_key,
        _ => bail!("owner_key is required when owner_scope is not user"),
    };
    Ok((
        owner_scope.to_string(),
        owner_key.to_string(),
        project.to_string(),
    ))
}

fn validate_owner_scope(owner_scope: &str) -> Result<()> {
    if matches!(owner_scope, "user" | "workspace" | "repo" | "session") {
        return Ok(());
    }
    bail!("unsupported user-context owner scope: {owner_scope}");
}

fn encode_ids(ids: &[i64]) -> Result<String> {
    if ids.iter().any(|id| *id <= 0) {
        bail!("source ids must be positive integers");
    }
    serde_json::to_string(ids).context("encode source ids")
}

fn parse_ids(label: &str, json: &str) -> Result<Vec<i64>> {
    let ids: Vec<i64> = serde_json::from_str(json)
        .with_context(|| format!("parse {label} as JSON integer array"))?;
    if ids.iter().any(|id| *id <= 0) {
        bail!("{label} must contain only positive integer ids");
    }
    Ok(ids)
}

fn encode_activity_refs(refs: &[ActivityRef]) -> Result<String> {
    if refs
        .iter()
        .any(|item| item.id <= 0 || item.kind.trim().is_empty())
    {
        bail!("activity source refs require a positive id and kind");
    }
    serde_json::to_string(refs).context("encode activity source refs")
}

fn parse_activity_refs(json: &str) -> Result<Vec<ActivityRef>> {
    let refs: Vec<ActivityRef> =
        serde_json::from_str(json).context("parse activity refs as JSON array")?;
    if refs
        .iter()
        .any(|item| item.id <= 0 || item.kind.trim().is_empty())
    {
        bail!("activity source refs require a positive id and kind");
    }
    Ok(refs)
}

fn summary_from_row(row: SummaryRow) -> Result<UserContextSummary> {
    Ok(UserContextSummary {
        id: row.id,
        user_key: row.user_key,
        owner_scope: row.owner_scope,
        owner_key: row.owner_key,
        scope: row.scope,
        scope_key: row.scope_key,
        summary_text: row.summary_text,
        source_claim_ids: parse_ids("source_claim_ids_json", &row.source_claim_ids_json)?,
        source_memory_ids: parse_ids("source_memory_ids_json", &row.source_memory_ids_json)?,
        source_activity_refs: parse_activity_refs(&row.source_activity_refs_json)?,
        status: row.status,
        model: row.model,
        version: row.version,
        created_at_epoch: row.created_at_epoch,
        updated_at_epoch: row.updated_at_epoch,
    })
}

fn map_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SummaryRow> {
    Ok(SummaryRow {
        id: row.get(0)?,
        user_key: row.get(1)?,
        owner_scope: row.get(2)?,
        owner_key: row.get(3)?,
        scope: row.get(4)?,
        scope_key: row.get(5)?,
        summary_text: row.get(6)?,
        source_claim_ids_json: row.get(7)?,
        source_memory_ids_json: row.get(8)?,
        source_activity_refs_json: row.get(9)?,
        status: row.get(10)?,
        model: row.get(11)?,
        version: row.get(12)?,
        created_at_epoch: row.get(13)?,
        updated_at_epoch: row.get(14)?,
    })
}

fn compact_line(text: &str, max_chars: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut out = text
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push('…');
    out
}

#[cfg(test)]
mod tests;
