use rusqlite::Connection;

use super::super::{query_candidate_promotion_stats, CandidatePromotionStat};
use super::setup_stats_schema;

#[test]
fn query_candidate_promotion_stats_groups_by_status_and_block_reason() -> anyhow::Result<()> {
    let conn = Connection::open_in_memory()?;
    setup_stats_schema(&conn);

    let now = 10_000_000;
    let recent = now - 1_000;
    let old = now - 8 * 24 * 3600;
    conn.execute_batch(&format!(
        "INSERT INTO memory_candidates (source_kind, review_status, auto_promote_block_reason, created_at_epoch) VALUES
            ('observation', 'auto_promoted', NULL, {recent}),
            ('observation', 'auto_promoted', NULL, {old}),
            ('observation', 'auto_promoted', NULL, {old}),
            ('observation', 'pending_review', 'no_supporting_source_observation', {recent}),
            ('observation', 'pending_review', 'no_supporting_source_observation', {recent}),
            ('observation', 'pending_review', 'no_supporting_source_observation', {old}),
            ('observation', 'pending_review', 'no_supporting_source_observation', {old}),
            ('summary', 'pending_review', 'summary_gate_shadow', {recent}),
            ('unattributed', 'pending_review', 'confidence_below_threshold', {old});"
    ))?;

    let stats = query_candidate_promotion_stats(&conn, now)?;

    assert_eq!(
        stats,
        vec![
            CandidatePromotionStat {
                source_kind: "observation".to_string(),
                review_status: "pending_review".to_string(),
                block_reason: Some("no_supporting_source_observation".to_string()),
                total: 4,
                last_7_days: 2,
            },
            CandidatePromotionStat {
                source_kind: "observation".to_string(),
                review_status: "auto_promoted".to_string(),
                block_reason: None,
                total: 3,
                last_7_days: 1,
            },
            CandidatePromotionStat {
                source_kind: "summary".to_string(),
                review_status: "pending_review".to_string(),
                block_reason: Some("summary_gate_shadow".to_string()),
                total: 1,
                last_7_days: 1,
            },
            CandidatePromotionStat {
                source_kind: "unattributed".to_string(),
                review_status: "pending_review".to_string(),
                block_reason: Some("confidence_below_threshold".to_string()),
                total: 1,
                last_7_days: 0,
            },
        ]
    );
    Ok(())
}
