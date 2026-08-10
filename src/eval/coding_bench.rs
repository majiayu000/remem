mod artifact;
mod audit_contract;
mod condition;
mod dry_run;
mod failure;
mod fixture;
mod isolation;
mod run_plan;
mod runner;
mod score;
#[cfg(test)]
mod tests;
mod types;

pub use artifact::{
    build_remem_contract_snapshot, validate_contract_snapshots, CodingBenchCondition,
    CodingBenchConditionReport, CodingBenchMemoryContractStatus, CodingBenchReport,
    CodingBenchRunMetrics, CodingBenchRunReport, CodingBenchRunScoreEvidence,
    CodingBenchScoreCommandEvidence, RememContractHealth, RememContractSnapshot,
    RememContractWarning, RememInjectedMemoryAuditSnapshot, RememStalenessHandlingSnapshot,
    RememTemporalFactEligibilitySnapshot, RememUsageFeedbackCoverageSnapshot,
    CODING_AGENT_AB_SPEC_PATH, CURRENT_MEMORY_CONTRACT_SPEC_PATH, MIN_RUNS_PER_CONDITION,
};
pub(crate) use audit_contract::verify_snapshot_against_persisted_injection;
pub use audit_contract::{
    verify_context_audit_snapshot, RememContextAuditSnapshot, RememContextAuditStatus,
};
pub use runner::{dry_run_plan, run_coding_bench};
pub use types::CodingBenchOptions;
