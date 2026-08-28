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

pub(crate) fn replay_trusted_security_snapshot_identity(
    task: &types::MemoryBenchTask,
) -> anyhow::Result<crate::eval::security_snapshot_identity::SnapshotIdentity> {
    let task = task.clone();
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(production_pipeline::trusted_snapshot_identity(&task))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("trusted security snapshot worker panicked"))?
}
