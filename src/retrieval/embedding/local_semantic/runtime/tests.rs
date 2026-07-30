use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Barrier, Mutex,
};

use super::*;

static RUNTIME_FIXTURE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[test]
fn process_cache_singleflights_one_model_build_per_artifact() -> Result<()> {
    let cache = Arc::new(Mutex::new(ProcessModelCache::<usize>::new(2)));
    let barrier = Arc::new(Barrier::new(8));
    let loads = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..8 {
        let cache = Arc::clone(&cache);
        let barrier = Arc::clone(&barrier);
        let loads = Arc::clone(&loads);
        handles.push(std::thread::spawn(move || -> Result<usize> {
            barrier.wait();
            let mut cache = cache
                .lock()
                .map_err(|_| anyhow::anyhow!("test model cache lock poisoned"))?;
            cache.with_model(
                LocalModelCacheKey {
                    preset: LocalEmbeddingPreset::MultilingualE5Small,
                    install_dir: PathBuf::from("/tmp/remem-process-cache-test"),
                },
                "artifact-a",
                || {
                    loads.fetch_add(1, Ordering::SeqCst);
                    Ok(41)
                },
                |model| {
                    *model += 1;
                    Ok(*model)
                },
            )
        }));
    }

    let mut outputs = Vec::new();
    for handle in handles {
        let output = handle
            .join()
            .map_err(|_| anyhow::anyhow!("cache test thread panicked"))??;
        outputs.push(output);
    }
    outputs.sort_unstable();
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert_eq!(outputs, (42..=49).collect::<Vec<_>>());
    Ok(())
}

#[test]
fn artifact_change_replaces_cached_model_once() -> Result<()> {
    let mut cache = ProcessModelCache::<usize>::new(2);
    let key = LocalModelCacheKey {
        preset: LocalEmbeddingPreset::MultilingualE5Small,
        install_dir: PathBuf::from("/tmp/remem-artifact-cache-test"),
    };
    let loads = AtomicUsize::new(0);

    for artifact in ["artifact-a", "artifact-a", "artifact-b", "artifact-b"] {
        cache.with_model(
            key.clone(),
            artifact,
            || {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(0)
            },
            |_| Ok(()),
        )?;
    }

    assert_eq!(loads.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn verified_model_load_file_disappearance_is_typed_unavailable() -> Result<()> {
    let root = std::env::temp_dir().join(format!(
        "remem-runtime-load-race-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let preset = LocalEmbeddingPreset::MultilingualE5Small;
    super::super::test_support::install_test_model_for_preset(&root, preset)?;
    let install_dir = std::fs::canonicalize(root.join(preset.model_id()))?;
    let manifest: LocalModelManifest = serde_json::from_slice(&std::fs::read(
        install_dir.join(super::super::MANIFEST_FILE),
    )?)?;
    let (_, model_path) =
        verified_runtime_file(&install_dir, &manifest, preset, preset.model_file())?;
    std::fs::remove_file(model_path)?;

    let error = with_verified_model(preset, &install_dir, &manifest, &"f".repeat(64), |_| Ok(()))
        .unwrap_err();

    assert!(
        super::super::is_model_unavailable_error(&error),
        "{error:#}"
    );
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn bge_uses_verified_file_backed_runtime_instead_of_loading_external_data_bytes() {
    assert_eq!(
        runtime_load_strategy(LocalEmbeddingPreset::MultilingualE5Small),
        RuntimeLoadStrategy::VerifiedBytes
    );
    assert_eq!(
        runtime_load_strategy(LocalEmbeddingPreset::BgeM3),
        RuntimeLoadStrategy::VerifiedFileBackedCache
    );
}

struct RuntimeFixture {
    root: PathBuf,
    install_dir: PathBuf,
    preset: LocalEmbeddingPreset,
    manifest: LocalModelManifest,
}

impl RuntimeFixture {
    fn bge() -> Result<Self> {
        let sequence = RUNTIME_FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "remem-bge-private-runtime-test-{}-{}-{sequence}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let preset = LocalEmbeddingPreset::BgeM3;
        super::super::test_support::install_test_model_for_preset(&root, preset)?;
        let install_dir = root.join(preset.model_id());
        let manifest = serde_json::from_slice(&std::fs::read(
            install_dir.join(super::super::MANIFEST_FILE),
        )?)?;
        Ok(Self {
            root,
            install_dir,
            preset,
            manifest,
        })
    }

    fn source_runtime_file(&self, runtime_file: &str) -> PathBuf {
        let revision = "a".repeat(40);
        self.install_dir
            .join(self.preset.cache_repo_dir())
            .join("snapshots")
            .join(revision)
            .join(runtime_file)
    }

    fn private_snapshot(&self, private_root: &Path, artifact_sha256: &str) -> PathBuf {
        private_root
            .join(self.preset.cache_repo_dir())
            .join("snapshots")
            .join(artifact_sha256)
    }

    fn private_runtime_file(
        &self,
        private_root: &Path,
        artifact_sha256: &str,
        runtime_file: &str,
    ) -> PathBuf {
        self.private_snapshot(private_root, artifact_sha256)
            .join(runtime_file)
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "remove BGE private runtime fixture {}: {error}",
                    self.root.display()
                );
            }
        }
    }
}

struct TestEnvVar {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl TestEnvVar {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for TestEnvVar {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[test]
fn bge_private_cache_survives_source_ref_and_file_race() -> Result<()> {
    let fixture = RuntimeFixture::bge()?;
    let artifact_sha256 = "b".repeat(64);
    let original_external_data = fixture.source_runtime_file("onnx/model.onnx_data");
    let original_external_data_content = std::fs::read(&original_external_data)?;
    let original_ref = fixture
        .install_dir
        .join(fixture.preset.cache_repo_dir())
        .join("refs/main");
    let loader_called = AtomicUsize::new(0);

    let (_, private_cache) = load_verified_file_backed_model_with(
        fixture.preset,
        &fixture.install_dir,
        &fixture.manifest,
        &artifact_sha256,
        |private_root| {
            loader_called.fetch_add(1, Ordering::SeqCst);
            std::fs::remove_file(&original_ref)?;
            std::fs::write(&original_external_data, b"raced-source-content")?;

            let private_snapshot = fixture.private_snapshot(private_root, &artifact_sha256);
            let private_external_data = private_snapshot.join("onnx/model.onnx_data");
            assert_eq!(
                std::fs::read(private_external_data)?,
                original_external_data_content
            );
            for runtime_file in fixture.preset.required_runtime_files() {
                let metadata = std::fs::symlink_metadata(private_snapshot.join(runtime_file))?;
                assert!(metadata.file_type().is_file());
                assert!(!metadata.file_type().is_symlink());
            }
            Ok(())
        },
    )?;

    assert_eq!(loader_called.load(Ordering::SeqCst), 1);
    assert_ne!(private_cache.root(), fixture.install_dir);
    Ok(())
}

#[test]
fn bge_missing_source_fails_before_local_loader() -> Result<()> {
    let fixture = RuntimeFixture::bge()?;
    std::fs::remove_file(fixture.source_runtime_file("onnx/model.onnx_data"))?;
    let loader_called = AtomicUsize::new(0);

    let error = load_verified_file_backed_model_with(
        fixture.preset,
        &fixture.install_dir,
        &fixture.manifest,
        &"c".repeat(64),
        |_| {
            loader_called.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .unwrap_err();

    assert_eq!(loader_called.load(Ordering::SeqCst), 0);
    assert!(error.to_string().contains("model.onnx_data"), "{error:#}");
    Ok(())
}

#[test]
fn bge_private_cache_disappearance_after_preverify_fails_loudly() -> Result<()> {
    let fixture = RuntimeFixture::bge()?;
    let artifact_sha256 = "d".repeat(64);
    let loader_called = AtomicUsize::new(0);

    let error = load_verified_file_backed_model_with(
        fixture.preset,
        &fixture.install_dir,
        &fixture.manifest,
        &artifact_sha256,
        |private_root| {
            loader_called.fetch_add(1, Ordering::SeqCst);
            let external_data = fixture.private_runtime_file(
                private_root,
                &artifact_sha256,
                "onnx/model.onnx_data",
            );
            remove_test_file(&external_data)?;
            Ok(())
        },
    )
    .unwrap_err();

    assert_eq!(loader_called.load(Ordering::SeqCst), 1, "{error:#}");
    assert!(
        format!("{error:#}").contains("verify private runtime cache after model initialization"),
        "{error:#}"
    );
    assert!(
        format!("{error:#}").contains("missing one or more expected files"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn bge_private_cache_change_after_preverify_fails_loudly() -> Result<()> {
    let fixture = RuntimeFixture::bge()?;
    let artifact_sha256 = "e".repeat(64);
    let loader_called = AtomicUsize::new(0);

    let error = load_verified_file_backed_model_with(
        fixture.preset,
        &fixture.install_dir,
        &fixture.manifest,
        &artifact_sha256,
        |private_root| {
            loader_called.fetch_add(1, Ordering::SeqCst);
            let external_data = fixture.private_runtime_file(
                private_root,
                &artifact_sha256,
                "onnx/model.onnx_data",
            );
            remove_test_file(&external_data)?;
            std::fs::write(&external_data, b"changed-after-preverify")?;
            Ok(())
        },
    )
    .unwrap_err();

    assert_eq!(loader_called.load(Ordering::SeqCst), 1);
    assert!(
        format!("{error:#}").contains("verify private runtime cache after model initialization"),
        "{error:#}"
    );
    assert!(
        format!("{error:#}").contains("model.onnx_data"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn bge_ambient_hf_home_does_not_change_private_loader_root() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let ambient_hf_home = std::env::temp_dir().join("remem-untrusted-hf-home");
    let _hf_home = TestEnvVar::set("HF_HOME", &ambient_hf_home);
    let fixture = RuntimeFixture::bge()?;
    let loader_called = AtomicUsize::new(0);

    let (_, private_cache) = load_verified_file_backed_model_with(
        fixture.preset,
        &fixture.install_dir,
        &fixture.manifest,
        &"f".repeat(64),
        |private_root| {
            loader_called.fetch_add(1, Ordering::SeqCst);
            assert_ne!(private_root, ambient_hf_home);
            assert_eq!(
                std::env::var_os("HF_HOME").as_deref(),
                Some(ambient_hf_home.as_os_str())
            );
            Ok(())
        },
    )?;

    assert_eq!(loader_called.load(Ordering::SeqCst), 1);
    assert_ne!(private_cache.root(), ambient_hf_home);
    Ok(())
}

#[test]
fn bge_runtime_sources_have_no_network_capable_model_loader() {
    let file_backed_source = include_str!("file_backed_text.rs");
    let sources = [include_str!("../runtime.rs"), file_backed_source];
    let forbidden = [
        ["TextEmbedding", "::try_new("].concat(),
        ["Api", "Repo"].concat(),
        ["hf", "_hub"].concat(),
        ["pull_", "from_hf"].concat(),
        ["req", "west"].concat(),
        ["u", "req"].concat(),
    ];

    for source in sources {
        for symbol in &forbidden {
            assert!(
                !source.contains(symbol),
                "BGE runtime source must not contain network-capable symbol {symbol}"
            );
        }
    }
    assert!(
        file_backed_source.contains(".commit_from_file(model_path)"),
        "BGE runtime must keep split external model data file-backed"
    );
}

fn remove_test_file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)?;
    }
    std::fs::remove_file(path)?;
    Ok(())
}
