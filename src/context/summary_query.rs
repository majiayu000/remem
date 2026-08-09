use std::collections::HashSet;

use anyhow::Result;
use rusqlite::Connection;

use super::memory_selection::{
    context_cluster_suffix, normalize_cluster_text, reference_cluster_key,
};
use super::memory_traits::is_self_diagnostic_text;
use super::types::{ContextPreselectionDrop, ContextPreselectionItem, SessionSummaryBrief};

const SUMMARY_FETCH_BATCH_SIZE: usize = 25;
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
    let mut offset = 0usize;

    while selected.len() < limit && offset < scan_limit {
        let fetch_limit = SUMMARY_FETCH_BATCH_SIZE.min(scan_limit - offset);
        let batch = query_summary_batch(conn, project, fetch_limit, offset)?;
        if batch.is_empty() {
            break;
        }

        for row in batch {
            let summary = row.summary;
            if !crate::db::summary_poisoning::summary_injectable(
                conn,
                summary.id,
                &[
                    ("request", Some(summary.request.as_str())),
                    ("completed", summary.completed.as_deref()),
                ],
                "context_recent_sessions",
            ) {
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

        offset += fetch_limit;
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
}

fn query_summary_batch(
    conn: &Connection,
    project: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<SessionSummaryQueryRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT ss.id, \
             CASE \
               WHEN ss.request LIKE 'Captured event range %..%' THEN \
                 COALESCE(NULLIF(ss.decisions, ''), NULLIF(ss.learned, ''), \
                          NULLIF(ss.next_steps, ''), NULLIF(ss.preferences, ''), \
                          NULLIF(ss.completed, ''), ss.request) \
               ELSE ss.request \
             END AS display_request, \
             ss.completed, \
             ss.created_at_epoch, \
             CASE \
               WHEN ss.session_row_id IS NOT NULL AND s.session_id IS NOT NULL THEN \
                 'mem-' || substr(s.session_id, 1, 8) \
               ELSE ss.memory_session_id \
             END AS session_key \
         FROM session_summaries ss \
         LEFT JOIN sessions s ON s.id = ss.session_row_id \
         WHERE ss.request IS NOT NULL AND ss.request != '' \
           AND COALESCE(ss.poisoning_status, 'legacy_unscanned') != 'quarantined' \
           AND (ss.session_row_id IS NULL \
                OR ss.request NOT LIKE 'Captured event range %..%' \
                OR COALESCE(ss.decisions, '') != '' \
                OR COALESCE(ss.learned, '') != '' \
                OR COALESCE(ss.next_steps, '') != '' \
                OR COALESCE(ss.preferences, '') != '') \
           AND ((ss.owner_scope = 'repo' AND ss.owner_key = ?1) \
                OR (ss.owner_scope = 'repo' AND ss.target_project = ?1) \
                OR (ss.owner_scope IS NULL AND ss.project = ?1)) \
         ORDER BY ss.created_at_epoch DESC, display_request ASC, ss.completed ASC LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(
        rusqlite::params![project, limit as i64, offset as i64],
        |row| {
            Ok(SessionSummaryQueryRow {
                summary: SessionSummaryBrief {
                    id: row.get(0)?,
                    request: row.get(1)?,
                    completed: row.get(2)?,
                    created_at_epoch: row.get(3)?,
                },
                session_key: row.get(4)?,
            })
        },
    )?;
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
