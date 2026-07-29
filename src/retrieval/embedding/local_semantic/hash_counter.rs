use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{bail, Context, Result};

static MODEL_FILE_HASH_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();

pub(super) struct ModelFileHashCounter {
    canonical_root: PathBuf,
}

impl ModelFileHashCounter {
    pub(super) fn start(root: &Path) -> Result<Self> {
        let canonical_root = std::fs::canonicalize(root)
            .with_context(|| format!("canonicalize model hash counter root {}", root.display()))?;
        let mut counts = model_file_hash_counts()
            .lock()
            .map_err(|_| anyhow::anyhow!("model file hash counter lock poisoned"))?;
        if counts.keys().any(|registered| {
            registered.starts_with(&canonical_root) || canonical_root.starts_with(registered)
        }) {
            bail!(
                "model file hash counter root overlaps an active counter: {}",
                canonical_root.display()
            );
        }
        counts.insert(canonical_root.clone(), 0);
        Ok(Self { canonical_root })
    }

    pub(super) fn count(&self) -> Result<usize> {
        model_file_hash_counts()
            .lock()
            .map_err(|_| anyhow::anyhow!("model file hash counter lock poisoned"))?
            .get(&self.canonical_root)
            .copied()
            .with_context(|| {
                format!(
                    "model file hash counter is not registered for {}",
                    self.canonical_root.display()
                )
            })
    }
}

impl Drop for ModelFileHashCounter {
    fn drop(&mut self) {
        let mut counts = model_file_hash_counts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        counts.remove(&self.canonical_root);
    }
}

pub(super) struct PendingModelFileHash {
    roots: Vec<PathBuf>,
}

impl PendingModelFileHash {
    pub(super) fn for_path(path: &Path) -> Result<Self> {
        let canonical_path = std::fs::canonicalize(path)
            .with_context(|| format!("canonicalize tracked model file {}", path.display()))?;
        let counts = model_file_hash_counts()
            .lock()
            .map_err(|_| anyhow::anyhow!("model file hash counter lock poisoned"))?;
        let roots = counts
            .keys()
            .filter(|root| canonical_path.starts_with(root))
            .cloned()
            .collect();
        Ok(Self { roots })
    }

    pub(super) fn record(self) -> Result<()> {
        let mut counts = model_file_hash_counts()
            .lock()
            .map_err(|_| anyhow::anyhow!("model file hash counter lock poisoned"))?;
        for root in self.roots {
            let count = counts.get_mut(&root).with_context(|| {
                format!("model file hash counter disappeared for {}", root.display())
            })?;
            *count += 1;
        }
        Ok(())
    }
}

fn model_file_hash_counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
    MODEL_FILE_HASH_COUNTS.get_or_init(|| Mutex::new(HashMap::new()))
}
