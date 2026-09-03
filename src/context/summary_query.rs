use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use super::memory_selection::{
    context_cluster_suffix, normalize_cluster_text, reference_cluster_key,
};
use super::memory_traits::is_self_diagnostic_text;
use super::types::{ContextPreselectionDrop, ContextPreselectionItem, SessionSummaryBrief};

const SUMMARY_MAX_SCAN: usize = 200;
const STALE_DESIGN_SUMMARY_DAYS: i64 = 7;

pub(super) struct SummarySelection {
    pub selected: Vec<SessionSummaryBrief>,
    pub poisoning_drops: Vec<SessionSummaryBrief>,
    pub preselection_drops: Vec<ContextPreselectionDrop>,
}

#[cfg(test)]
pub(super) fn query_recent_summaries(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<Vec<SessionSummaryBrief>> {
    Ok(query_recent_summaries_with_drops(conn, project, limit)?.selected)
}

pub(super) fn query_recent_summaries_with_drops(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<SummarySelection> {
    query_recent_summaries_with_drops_matching(conn, project, limit, false)
}

pub(super) fn query_recent_unfinished_summaries_with_drops(
    conn: &Connection,
    project: &str,
    limit: usize,
) -> Result<SummarySelection> {
    query_recent_summaries_with_drops_matching(conn, project, limit, true)
}

fn query_recent_summaries_with_drops_matching(
    conn: &Connection,
    project: &str,
    limit: usize,
    require_next_steps: bool,
) -> Result<SummarySelection> {
    if limit == 0 {
        return Ok(SummarySelection {
            selected: Vec::new(),
            poisoning_drops: Vec::new(),
            preselection_drops: Vec::new(),
        });
    }

    let scan_limit = SUMMARY_MAX_SCAN.max(limit);
    let now_epoch = chrono::Utc::now().timestamp();
    let mut selected = Vec::new();
    let mut low_signal_fallback = Vec::new();
    let mut poisoning_drops = Vec::new();
    let mut preselection_drops = Vec::new();
    let mut seen_clusters = HashSet::new();
    let mut seen_session_keys = HashSet::new();
    let mut candidates = query_summary_batch(conn, project, scan_limit + 1, require_next_steps)?;
    let has_more = candidates.len() > scan_limit;
    candidates.truncate(scan_limit);
    for row in candidates {
        if selected.len() >= limit {
            break;
        }
        let injectable = if require_next_steps {
            crate::db::summary_poisoning::summary_injectable(
                conn,
                row.summary.id,
                &[
                    ("request", row.source_request.as_deref()),
                    ("completed", row.summary.completed.as_deref()),
                    ("decisions", row.decisions.as_deref()),
                    ("learned", row.learned.as_deref()),
                    ("next_steps", row.next_steps.as_deref()),
                    ("preferences", row.preferences.as_deref()),
                ],
                "prompt_submit_continuity",
            )
        } else {
            crate::db::summary_poisoning::summary_injectable(
                conn,
                row.summary.id,
                &[
                    ("request", Some(row.summary.request.as_str())),
                    ("completed", row.summary.completed.as_deref()),
                ],
                "context_recent_sessions",
            )
        };
        let summary = row.summary;
        if !injectable {
            poisoning_drops.push(summary);
            continue;
        }
        if is_session_summary_self_diagnostic(&summary) {
            preselection_drops.push(summary_drop(summary, "summary_self_diagnostic"));
            continue;
        }

        let cluster_key = summary_cluster_key(&summary);
        if seen_clusters.contains(&cluster_key) {
            preselection_drops.push(summary_drop(summary, "summary_cluster_dedup"));
            continue;
        }
        if row
            .session_key
            .as_ref()
            .is_some_and(|session_key| seen_session_keys.contains(session_key))
        {
            preselection_drops.push(summary_drop(summary, "summary_session_dedup"));
            continue;
        }

        if is_stale_design_prototype_summary(&summary, now_epoch) {
            low_signal_fallback.push((cluster_key, row.session_key, summary));
            continue;
        }
        if selected.len() >= limit {
            preselection_drops.push(summary_drop(summary, "summary_item_limit"));
            continue;
        }

        seen_clusters.insert(cluster_key);
        if let Some(session_key) = row.session_key {
            seen_session_keys.insert(session_key);
        }
        selected.push(summary);
    }

    if selected.is_empty() {
        for (cluster_key, session_key, summary) in low_signal_fallback {
            if seen_clusters.contains(&cluster_key) {
                preselection_drops.push(summary_drop(summary, "summary_cluster_dedup"));
                continue;
            }
            if session_key
                .as_ref()
                .is_some_and(|key| seen_session_keys.contains(key))
            {
                preselection_drops.push(summary_drop(summary, "summary_session_dedup"));
                continue;
            }
            if selected.len() >= limit {
                preselection_drops.push(summary_drop(summary, "summary_item_limit"));
                continue;
            }
            seen_clusters.insert(cluster_key);
            if let Some(session_key) = session_key {
                seen_session_keys.insert(session_key);
            }
            selected.push(summary);
        }
    } else {
        preselection_drops.extend(
            low_signal_fallback
                .into_iter()
                .map(|(_, _, summary)| summary_drop(summary, "summary_stale_design_fallback")),
        );
    }

    if selected.len() < limit && has_more {
        anyhow::bail!(
            "summary continuity scan budget exhausted after {scan_limit} rows before finding {limit} safe anchors"
        );
    }

    Ok(SummarySelection {
        selected,
        poisoning_drops,
        preselection_drops,
    })
}

fn summary_drop(summary: SessionSummaryBrief, reason: &'static str) -> ContextPreselectionDrop {
    ContextPreselectionDrop {
        item: ContextPreselectionItem::Summary(summary),
        reason,
    }
}

struct SessionSummaryQueryRow {
    summary: SessionSummaryBrief,
    session_key: Option<String>,
    source_request: Option<String>,
    decisions: Option<String>,
    learned: Option<String>,
    next_steps: Option<String>,
    preferences: Option<String>,
}

fn query_summary_batch(
    conn: &Connection,
    project: &str,
    limit: usize,
    require_next_steps: bool,
) -> Result<Vec<SessionSummaryQueryRow>> {
    let epoch_secs_only = crate::db::query::EPOCH_SECS_ONLY;
    let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let (owner_clause, idx) = crate::project_alias::push_project_value_filter(
        conn,
        "ss.owner_key",
        project,
        1,
        &mut params_vec,
    )?;
    let (target_clause, idx) = crate::project_alias::push_project_value_filter(
        conn,
        "ss.target_project",
        project,
        idx,
        &mut params_vec,
    )?;
    let (legacy_clause, idx) = crate::project_alias::push_project_value_filter(
        conn,
        "ss.project",
        project,
        idx,
        &mut params_vec,
    )?;
    let limit_idx = idx;
    params_vec.push(Box::new(limit as i64));
    let next_steps_idx = limit_idx + 1;
    params_vec.push(Box::new(i64::from(require_next_steps)));
    let sql = format!(
        "SELECT ss.id, \
             CASE \
               WHEN ss.request LIKE 'Captured event range %..%' THEN \
                 COALESCE(NULLIF(TRIM(ss.decisions), ''), NULLIF(TRIM(ss.learned), ''), \
                          NULLIF(TRIM(ss.next_steps), ''), NULLIF(TRIM(ss.preferences), ''), \
                          NULLIF(TRIM(ss.completed), ''), ss.request) \
               ELSE COALESCE(NULLIF(TRIM(ss.request), ''), NULLIF(TRIM(ss.next_steps), ''), \
                             NULLIF(TRIM(ss.completed), ''), NULLIF(TRIM(ss.decisions), ''), \
                             NULLIF(TRIM(ss.learned), ''), NULLIF(TRIM(ss.preferences), ''), \
                             'Session summary #' || ss.id) \
             END AS display_request, \
             ss.completed, \
             ss.created_at_epoch, \
             CASE \
               WHEN ss.session_row_id IS NOT NULL AND s.session_id IS NOT NULL THEN \
                 'mem-' || substr(s.session_id, 1, 8) \
               ELSE ss.memory_session_id \
             END AS session_key, \
             ss.request, \
             ss.decisions, \
             ss.learned, \
             ss.next_steps, \
             ss.preferences \
         FROM session_summaries ss \
         LEFT JOIN sessions s ON s.id = ss.session_row_id \
         WHERE ss.{epoch_secs_only} \
           AND ((?{next_steps_idx} = 0 AND NULLIF(TRIM(ss.request), '') IS NOT NULL) \
                OR (?{next_steps_idx} = 1 AND NULLIF(TRIM(ss.next_steps), '') IS NOT NULL)) \
           AND COALESCE(ss.poisoning_status, 'legacy_unscanned') != 'quarantined' \
           AND (ss.session_row_id IS NULL \
                OR ss.request NOT LIKE 'Captured event range %..%' \
                OR COALESCE(ss.decisions, '') != '' \
                OR COALESCE(ss.learned, '') != '' \
                OR COALESCE(ss.next_steps, '') != '' \
                OR COALESCE(ss.preferences, '') != '') \
           AND ((ss.owner_scope = 'repo' AND {owner_clause}) \
                OR (ss.owner_scope = 'repo' AND {target_clause}) \
                OR (ss.owner_scope IS NULL AND {legacy_clause})) \
         ORDER BY ss.created_at_epoch DESC, display_request ASC, ss.completed ASC \
         LIMIT ?{limit_idx}",
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let refs = crate::db::to_sql_refs(&params_vec);
    let rows = stmt.query_map(refs.as_slice(), |row| {
        Ok(SessionSummaryQueryRow {
            summary: SessionSummaryBrief {
                id: row.get(0)?,
                request: row.get(1)?,
                completed: row.get(2)?,
                created_at_epoch: row.get(3)?,
            },
            session_key: row.get(4)?,
            source_request: row.get(5)?,
            decisions: row.get(6)?,
            learned: row.get(7)?,
            next_steps: row.get(8)?,
            preferences: row.get(9)?,
        })
    })?;
    crate::db::query::collect_rows(rows)
}

fn is_session_summary_self_diagnostic(summary: &SessionSummaryBrief) -> bool {
    is_self_diagnostic_text(&session_summary_haystack(summary))
}

fn is_stale_design_prototype_summary(summary: &SessionSummaryBrief, now_epoch: i64) -> bool {
    let age_days = (now_epoch - summary.created_at_epoch) / 86400;
    if age_days <= STALE_DESIGN_SUMMARY_DAYS {
        return false;
    }
    ["landing page", "wireframe", "starfield"]
        .iter()
        .any(|needle| session_summary_haystack(summary).contains(needle))
}

fn summary_cluster_key(summary: &SessionSummaryBrief) -> String {
    let request = normalize_cluster_text(&summary.request);
    let tokens: Vec<&str> = request.split_whitespace().collect();
    if let Some(reference_key) = reference_cluster_key(&tokens) {
        return reference_key;
    }
    context_cluster_suffix(&request)
}

fn session_summary_haystack(summary: &SessionSummaryBrief) -> String {
    format!(
        "{} {}",
        summary.request,
        summary.completed.as_deref().unwrap_or_default()
    )
    .to_ascii_lowercase()
}
