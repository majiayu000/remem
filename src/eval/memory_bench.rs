mod baselines;
mod diagnostics;
mod fixture;
mod production_pipeline;
mod runner;
#[cfg(test)]
mod tests;
pub mod types;

pub use runner::{run_memory_bench, MemoryBenchOptions};
pub use types::{MemoryBenchCondition, MemoryBenchSuiteFixture};

pub(crate) struct VerifiedSecurityPolicyState {
    pub active_claim_count: u32,
    pub candidate_count: u32,
    pub summary_input_count: u32,
    pub poisoning_source_scanner_matched: bool,
    pub poisoning_generated_surface_blocked: bool,
}

pub(crate) fn score_verified_security_policy(
    condition: MemoryBenchCondition,
    task: &types::MemoryBenchTask,
    retrieved_event_ids: &[String],
    abstained: bool,
    state: VerifiedSecurityPolicyState,
) -> types::MemoryBenchPolicyOutcome {
    let measurement = types::MemoryBenchPolicyMeasurement {
        verification_path: "capture_observation_candidate_promotion".to_string(),
        measurement_source: "sqlite_production_tables".to_string(),
        source_scanner_config: "scan_source_instruction_pattern(include_opaque_payload=false); generated_surfaces=scan_instruction_pattern(include_opaque_payload=true)".to_string(),
        active_claim_count: state.active_claim_count,
        candidate_count: state.candidate_count,
        summary_input_count: state.summary_input_count,
        poisoning_source_scanner_matched: state.poisoning_source_scanner_matched,
        poisoning_generated_surface_blocked: state.poisoning_generated_surface_blocked,
    };
    diagnostics::score_policy(
        condition,
        task,
        retrieved_event_ids,
        abstained,
        Some(&measurement),
    )
}

pub(crate) fn summarize_verified_security_policy(
    outcomes: &[types::MemoryBenchPolicyOutcome],
) -> types::MemoryBenchPolicySummary {
    types::summarize_policy_outcomes(outcomes)
}

pub(crate) fn replay_trusted_security_snapshot_identity(
    task: &types::MemoryBenchTask,
) -> anyhow::Result<crate::eval::security_snapshot_identity::SnapshotIdentity> {
    #[cfg(test)]
    let replay_probe = production_pipeline::current_replay_probe();
    let task = task.clone();
    std::thread::spawn(move || {
        #[cfg(test)]
        let _replay_probe_guard = production_pipeline::attach_replay_probe(replay_probe);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(production_pipeline::trusted_snapshot_identity(&task))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("trusted security snapshot worker panicked"))?
}
