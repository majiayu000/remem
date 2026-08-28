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
