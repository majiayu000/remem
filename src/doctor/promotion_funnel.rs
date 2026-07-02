use std::collections::BTreeMap;

use rusqlite::Connection;

use crate::db;

pub(super) fn with_candidate_source_kind_detail(conn: &Connection, mut detail: String) -> String {
    let Some(source_detail) = candidate_source_kind_detail(conn) else {
        return detail;
    };
    detail.push_str("; ");
    detail.push_str(&source_detail);
    detail
}

fn candidate_source_kind_detail(conn: &Connection) -> Option<String> {
    let now = chrono::Utc::now().timestamp();
    let stats = db::query_candidate_promotion_stats(conn, now).ok()?;
    if stats.is_empty() {
        return None;
    }
    #[derive(Default)]
    struct SourceSummary {
        total: i64,
        pending: i64,
        shadow_would_promote: i64,
    }
    let mut by_source: BTreeMap<String, SourceSummary> = BTreeMap::new();
    for stat in stats {
        let entry = by_source.entry(stat.source_kind).or_default();
        entry.total += stat.total;
        if stat.review_status == "pending_review" {
            entry.pending += stat.total;
        }
        if stat.block_reason.as_deref() == Some("summary_gate_shadow") {
            entry.shadow_would_promote += stat.total;
        }
    }
    Some(format!(
        "candidate_sources={}",
        by_source
            .into_iter()
            .map(|(source, summary)| format!(
                "{}:total={},pending={},shadow_would_promote={}",
                source, summary.total, summary.pending, summary.shadow_would_promote
            ))
            .collect::<Vec<_>>()
            .join(",")
    ))
}
