use super::*;

#[test]
fn once_budget_caps_work_items_and_elapsed_time() {
    let start = Instant::now();
    let mut budget = WorkerRunBudget::new(true, start);

    assert_eq!(
        budget.remaining_work_items(start),
        ONCE_WORKER_MAX_WORK_ITEMS
    );
    budget.record_work_items(2);
    assert_eq!(
        budget.remaining_work_items(start),
        ONCE_WORKER_MAX_WORK_ITEMS - 2
    );
    assert_eq!(budget.exhaustion_reason(start), None);

    budget.record_work_items(ONCE_WORKER_MAX_WORK_ITEMS - 2);
    assert_eq!(budget.remaining_work_items(start), 0);
    assert_eq!(budget.exhaustion_reason(start), Some("work_item_limit"));

    let elapsed_budget = WorkerRunBudget::new(true, start);
    let deadline = start + ONCE_WORKER_MAX_ELAPSED;
    assert_eq!(elapsed_budget.remaining_work_items(deadline), 0);
    assert_eq!(
        elapsed_budget.exhaustion_reason(deadline),
        Some("elapsed_limit")
    );
}

#[test]
fn daemon_budget_is_unlimited() {
    let start = Instant::now();
    let mut budget = WorkerRunBudget::new(false, start);
    budget.record_work_items(usize::MAX);

    assert_eq!(
        budget.remaining_work_items(start + ONCE_WORKER_MAX_ELAPSED),
        usize::MAX
    );
    assert_eq!(
        budget.exhaustion_reason(start + ONCE_WORKER_MAX_ELAPSED),
        None
    );
}
