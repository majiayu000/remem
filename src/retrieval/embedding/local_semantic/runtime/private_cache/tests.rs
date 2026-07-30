use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Barrier,
};

use anyhow::{Context, Result};

use super::*;

static CACHE_FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct CacheFixture {
    root: PathBuf,
    install_dir: PathBuf,
    preset: LocalEmbeddingPreset,
    manifest: LocalModelManifest,
}

impl CacheFixture {
    fn new() -> Result<Self> {
        let sequence = CACHE_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "remem-private-runtime-cache-test-{}-{}-{sequence}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let preset = LocalEmbeddingPreset::BgeM3;
        super::super::super::test_support::install_test_model_for_preset(&root, preset)?;
        let install_dir = root.join(preset.model_id());
        let manifest = serde_json::from_slice(&std::fs::read(
            install_dir.join(super::super::super::MANIFEST_FILE),
        )?)?;
        Ok(Self {
            root,
            install_dir,
            preset,
            manifest,
        })
    }

    fn materialize(&self, digest: char) -> Result<PrivateRuntimeCache> {
        PrivateRuntimeCache::materialize(
            &self.install_dir,
            &self.manifest,
            self.preset,
            &digest.to_string().repeat(64),
        )
    }

    fn cache_parent(&self) -> PathBuf {
        self.install_dir.join(PRIVATE_RUNTIME_CACHE_DIR)
    }
}

impl Drop for CacheFixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "remove private runtime cache fixture {}: {error}",
                    self.root.display()
                );
            }
        }
    }
}

#[test]
fn repeated_materialization_reuses_one_deterministic_cache() -> Result<()> {
    let fixture = CacheFixture::new()?;
    let first = fixture.materialize('a')?;
    let second = fixture.materialize('a')?;

    assert_eq!(first.root(), second.root());
    assert_eq!(managed_cache_directories(&fixture.cache_parent())?.len(), 1);
    Ok(())
}

#[test]
fn stale_artifacts_are_collected_only_after_shared_usage_releases() -> Result<()> {
    let fixture = CacheFixture::new()?;
    let old = fixture.materialize('a')?;
    let old_path = old.root().to_path_buf();
    let newer = fixture.materialize('b')?;
    let newer_path = newer.root().to_path_buf();

    assert!(old_path.is_dir(), "active old cache must not be collected");
    drop(old);
    let newest = fixture.materialize('c')?;

    assert!(!old_path.exists(), "released old cache should be collected");
    assert!(
        newer_path.is_dir(),
        "cache with a live shared usage lock must be retained"
    );
    assert!(newest.root().is_dir());
    Ok(())
}

#[test]
fn concurrent_materialization_singleflights_to_one_cache_directory() -> Result<()> {
    let fixture = Arc::new(CacheFixture::new()?);
    let start = Arc::new(Barrier::new(8));
    let mut threads = Vec::new();

    for _ in 0..8 {
        let fixture = Arc::clone(&fixture);
        let start = Arc::clone(&start);
        threads.push(std::thread::spawn(
            move || -> Result<PrivateRuntimeCache> {
                start.wait();
                fixture.materialize('d')
            },
        ));
    }

    let mut caches = Vec::new();
    for thread in threads {
        caches.push(
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("private cache worker thread panicked"))??,
        );
    }

    let roots = caches
        .iter()
        .map(|cache| cache.root().to_path_buf())
        .collect::<Vec<_>>();
    assert!(roots.windows(2).all(|pair| pair[0] == pair[1]));
    assert_eq!(managed_cache_directories(&fixture.cache_parent())?.len(), 1);
    Ok(())
}

fn managed_cache_directories(parent: &Path) -> Result<Vec<PathBuf>> {
    std::fs::read_dir(parent)
        .with_context(|| format!("read private runtime cache parent {}", parent.display()))?
        .filter_map(|entry| match entry {
            Ok(entry) => {
                let name = entry.file_name();
                let managed = name.to_str().is_some_and(super::is_managed_cache_name);
                managed.then_some(Ok(entry.path()))
            }
            Err(error) => Some(Err(error.into())),
        })
        .collect()
}
