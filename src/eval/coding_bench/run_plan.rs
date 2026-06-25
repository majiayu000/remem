use anyhow::{Context, Result};

use super::types::BenchCondition;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunPlanEntry {
    pub condition: BenchCondition,
    pub task_index: usize,
    pub run_index: usize,
}

pub fn randomized_run_plan(
    conditions: &[BenchCondition],
    task_count: usize,
    runs_per_condition: usize,
) -> Result<Vec<RunPlanEntry>> {
    let mut plan = build_run_plan(conditions, task_count, runs_per_condition);
    if plan.len() > 1 {
        let seed = random_seed().context("seed coding benchmark run order")?;
        shuffle_run_plan_with_seed(&mut plan, seed);
    }
    Ok(plan)
}

fn build_run_plan(
    conditions: &[BenchCondition],
    task_count: usize,
    runs_per_condition: usize,
) -> Vec<RunPlanEntry> {
    let mut plan = Vec::with_capacity(conditions.len() * task_count * runs_per_condition);
    for condition in conditions {
        for task_index in 0..task_count {
            for run_index in 1..=runs_per_condition {
                plan.push(RunPlanEntry {
                    condition: *condition,
                    task_index,
                    run_index,
                });
            }
        }
    }
    plan
}

fn random_seed() -> Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes)
        .map_err(|err| anyhow::anyhow!("getrandom failed while seeding run order: {err}"))?;
    let seed = u64::from_le_bytes(bytes);
    Ok(seed.max(1))
}

fn shuffle_run_plan_with_seed(plan: &mut [RunPlanEntry], seed: u64) {
    let mut state = seed;
    for i in (1..plan.len()).rev() {
        let j = (next_shuffle_u64(&mut state) % (i as u64 + 1)) as usize;
        plan.swap(i, j);
    }
}

fn next_shuffle_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn randomized_run_plan_keeps_complete_matrix_without_grouped_order() {
        let conditions = BenchCondition::ALL.to_vec();
        let sequential = build_run_plan(&conditions, 5, 3);
        let mut randomized = sequential.clone();
        shuffle_run_plan_with_seed(&mut randomized, 0x1234_5678_9ABC_DEF0);

        assert_ne!(
            randomized, sequential,
            "benchmark execution order should not stay grouped by condition"
        );

        let mut sorted_sequential = sequential;
        let mut sorted_randomized = randomized;
        sort_run_plan(&mut sorted_sequential);
        sort_run_plan(&mut sorted_randomized);
        assert_eq!(
            sorted_randomized, sorted_sequential,
            "randomization must preserve every condition/task/run tuple"
        );
    }

    fn sort_run_plan(plan: &mut [RunPlanEntry]) {
        plan.sort_by_key(|entry| {
            (
                condition_index(entry.condition),
                entry.task_index,
                entry.run_index,
            )
        });
    }

    fn condition_index(condition: BenchCondition) -> usize {
        BenchCondition::ALL
            .iter()
            .position(|candidate| *candidate == condition)
            .expect("condition should be in ALL")
    }
}
