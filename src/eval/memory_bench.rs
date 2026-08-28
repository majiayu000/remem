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

pub(crate) fn trusted_security_snapshot_identity(
    task: &types::MemoryBenchTask,
) -> anyhow::Result<crate::eval::security_snapshot_identity::SnapshotIdentity> {
    use std::collections::BTreeMap;
    use std::sync::{Mutex, OnceLock};

    use sha2::{Digest, Sha256};

    static IDENTITIES: OnceLock<
        Mutex<BTreeMap<String, crate::eval::security_snapshot_identity::SnapshotIdentity>>,
    > = OnceLock::new();
    let key = format!("{:x}", Sha256::digest(serde_json::to_vec(task)?));
    let identities = IDENTITIES.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut identities = identities
        .lock()
        .map_err(|_| anyhow::anyhow!("trusted security snapshot cache is poisoned"))?;
    if let Some(identity) = identities.get(&key) {
        return Ok(identity.clone());
    }
    let task = task.clone();
    let identity = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(production_pipeline::trusted_snapshot_identity(&task))
    })
    .join()
    .map_err(|_| anyhow::anyhow!("trusted security snapshot worker panicked"))??;
    identities.insert(key, identity.clone());
    Ok(identity)
}
