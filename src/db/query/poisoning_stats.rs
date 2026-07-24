use anyhow::Result;
use rusqlite::Connection;

use super::stats::PoisoningDefenseStats;

/// One-shot aggregate of the poisoning-defense counters surfaced by CLI
/// status, HTTP `/status`, and doctor (GH-855). Errors propagate so a broken
/// schema cannot masquerade as a healthy zero report.
pub fn query_poisoning_defense_stats(conn: &Connection) -> Result<PoisoningDefenseStats> {
    let (quarantined_summaries, legacy_unscanned_summaries, summary_block_count) = conn.query_row(
        "SELECT
                COALESCE(SUM(CASE WHEN poisoning_status = 'quarantined' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN poisoning_status = 'legacy_unscanned' THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(poisoning_block_count), 0)
             FROM session_summaries",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    Ok(PoisoningDefenseStats {
        pattern_set_version: crate::memory::poisoning::INSTRUCTION_PATTERN_SET_VERSION,
        quarantined_candidates: conn.query_row(
            "SELECT COUNT(*) FROM memory_candidates WHERE review_status = 'quarantined'",
            [],
            |row| row.get(0),
        )?,
        quarantined_summaries,
        legacy_unscanned_summaries,
        summary_block_count,
        quarantined_observations: conn.query_row(
            "SELECT COUNT(*) FROM observations WHERE status = 'poisoning_quarantined'",
            [],
            |row| row.get(0),
        )?,
        memory_injection_drops: conn.query_row(
            "SELECT COUNT(*) FROM memory_poisoning_injection_drops",
            [],
            |row| row.get(0),
        )?,
    })
}
