use super::admission::IntervalAdmission;
use super::*;

#[test]
fn once_schedule_admits_exactly_one_sweep() {
    let start = Instant::now();
    let mut schedule = IntervalAdmission::new(true, start, RETRIEVAL_ENRICHMENT_INTERVAL);

    assert!(schedule.is_due(start));
    schedule.record_attempt(start);
    assert!(!schedule.is_due(start));
    assert!(!schedule.is_due(start + RETRIEVAL_ENRICHMENT_INTERVAL));
}

#[test]
fn daemon_schedule_admits_at_most_once_per_interval() {
    let start = Instant::now();
    let mut schedule = IntervalAdmission::new(false, start, RETRIEVAL_ENRICHMENT_INTERVAL);

    assert!(schedule.is_due(start));
    schedule.record_attempt(start);
    assert!(!schedule.is_due(start + Duration::from_secs(59)));
    assert!(schedule.is_due(start + RETRIEVAL_ENRICHMENT_INTERVAL));
}
