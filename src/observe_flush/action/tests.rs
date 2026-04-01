use super::helpers::{collect_observation_titles, split_timeout_range};
use crate::memory_format::ParsedObservation;

#[test]
fn split_timeout_range_splits_evenly_when_possible() {
    let ranges = split_timeout_range(0, 6, 1).unwrap();
    assert_eq!(ranges, [(0, 3), (3, 6)]);
}

#[test]
fn split_timeout_range_returns_none_for_single_item_batch() {
    assert_eq!(split_timeout_range(0, 1, 1), None);
}

#[test]
fn collect_observation_titles_skips_missing_titles() {
    let observations = vec![
        ParsedObservation {
            obs_type: "discovery".to_string(),
            title: Some("First".to_string()),
            subtitle: None,
            facts: vec![],
            narrative: None,
            concepts: vec![],
            files_read: vec![],
            files_modified: vec![],
        },
        ParsedObservation {
            obs_type: "bugfix".to_string(),
            title: None,
            subtitle: None,
            facts: vec![],
            narrative: None,
            concepts: vec![],
            files_read: vec![],
            files_modified: vec![],
        },
    ];

    let titles = collect_observation_titles(&observations);
    assert_eq!(titles, vec!["First"]);
}
