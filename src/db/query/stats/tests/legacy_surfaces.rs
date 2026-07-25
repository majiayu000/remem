use super::super::LegacySurfaceStats;

pub(super) fn expected_fixture() -> Vec<LegacySurfaceStats> {
    vec![
        LegacySurfaceStats {
            surface: "observations".to_string(),
            disposition: "reclassify-current".to_string(),
            row_count: 2,
            last_write_epoch: Some(220),
            frozen_write_violations: 0,
        },
        LegacySurfaceStats {
            surface: "observations_fts".to_string(),
            disposition: "reclassify-current".to_string(),
            row_count: 1,
            last_write_epoch: None,
            frozen_write_violations: 0,
        },
        LegacySurfaceStats {
            surface: "session_summaries".to_string(),
            disposition: "keep".to_string(),
            row_count: 1,
            last_write_epoch: Some(230),
            frozen_write_violations: 0,
        },
        LegacySurfaceStats {
            surface: "pending_observations".to_string(),
            disposition: "retire".to_string(),
            row_count: 5,
            last_write_epoch: Some(500),
            frozen_write_violations: 4,
        },
        LegacySurfaceStats {
            surface: "summary_jobs".to_string(),
            disposition: "retire-summary-only".to_string(),
            row_count: 3,
            last_write_epoch: Some(305),
            frozen_write_violations: 2,
        },
    ]
}
