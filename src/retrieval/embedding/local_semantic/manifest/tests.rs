use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};

use super::*;
use crate::retrieval::embedding::local_semantic::{
    MODEL_DOWNLOAD_LOCK_FILE, MODEL_STATE_LOCK_FILE,
};

static FIXTURE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct SymlinkModelFixture {
    root: PathBuf,
    install_dir: PathBuf,
    preset: LocalEmbeddingPreset,
    revision: String,
}

impl SymlinkModelFixture {
    #[cfg(unix)]
    fn new() -> Result<Self> {
        Self::new_for_preset(LocalEmbeddingPreset::default())
    }

    #[cfg(unix)]
    fn new_for_preset(preset: LocalEmbeddingPreset) -> Result<Self> {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "remem-model-symlink-test-{}-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            preset.label()
        ));
        let install_dir = root.join(preset.model_id());
        let revision = "a".repeat(40);
        let repo_dir = install_dir.join(preset.cache_repo_dir());
        std::fs::create_dir_all(repo_dir.join("refs"))?;
        std::fs::create_dir_all(repo_dir.join("blobs"))?;
        std::fs::write(install_dir.join(MODEL_DOWNLOAD_LOCK_FILE), b"")?;
        std::fs::write(repo_dir.join("refs/main"), &revision)?;

        for (index, runtime_file) in preset.required_runtime_files().enumerate() {
            let blob_name = format!("fixture-blob-{index}");
            std::fs::write(
                repo_dir.join("blobs").join(&blob_name),
                format!("fixture:{runtime_file}"),
            )?;
            let pointer = repo_dir
                .join("snapshots")
                .join(&revision)
                .join(runtime_file);
            std::fs::create_dir_all(
                pointer
                    .parent()
                    .context("snapshot pointer should have a parent")?,
            )?;
            let target = if runtime_file.contains('/') {
                format!("../../../blobs/{blob_name}")
            } else {
                format!("../../blobs/{blob_name}")
            };
            symlink(target, pointer)?;
        }

        Ok(Self {
            root,
            install_dir,
            preset,
            revision,
        })
    }

    fn publish_manifest(&self) -> Result<LocalModelManifest> {
        let (files, symlinks) = collect_model_artifacts(&self.install_dir, self.preset)?;
        let manifest = LocalModelManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            preset: self.preset.label().to_string(),
            model_id: self.preset.model_id().to_string(),
            upstream_model: self.preset.upstream_model().to_string(),
            dimensions: self.preset.dimensions(),
            runtime: FASTEMBED_RUNTIME.to_string(),
            source_url: Some(self.preset.source_url()),
            downloaded_at_epoch: 1,
            files,
            symlinks,
        };
        write_manifest(&self.install_dir, &manifest)?;
        Ok(manifest)
    }

    #[cfg(unix)]
    fn publish_legacy_manifest(&self, omit_file_suffix: Option<&str>) -> Result<()> {
        let (mut files, symlinks) = collect_model_artifacts(&self.install_dir, self.preset)?;
        for symlink in symlinks {
            let target = files
                .iter()
                .find(|file| file.path == symlink.resolved_path)
                .with_context(|| {
                    format!(
                        "legacy fixture symlink {} should resolve to a collected file",
                        symlink.path
                    )
                })?;
            files.push(LocalModelFile {
                path: symlink.path,
                sha256: target.sha256.clone(),
                source_sha256: None,
                bytes: target.bytes,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        if let Some(suffix) = omit_file_suffix {
            files.retain(|file| !file.path.ends_with(suffix));
        }
        let legacy = serde_json::json!({
            "schema_version": 1,
            "preset": self.preset.label(),
            "model_id": self.preset.model_id(),
            "upstream_model": self.preset.upstream_model(),
            "dimensions": self.preset.dimensions(),
            "runtime": FASTEMBED_RUNTIME,
            "source_url": self.preset.source_url(),
            "downloaded_at_epoch": 1,
            "files": files,
        });
        std::fs::write(
            self.install_dir.join(MANIFEST_FILE),
            serde_json::to_vec_pretty(&legacy)?,
        )?;
        std::fs::remove_file(self.install_dir.join(MODEL_DOWNLOAD_LOCK_FILE))?;
        Ok(())
    }

    fn snapshot_path(&self, runtime_file: &str) -> PathBuf {
        self.install_dir
            .join(self.preset.cache_repo_dir())
            .join("snapshots")
            .join(&self.revision)
            .join(runtime_file)
    }
}

impl Drop for SymlinkModelFixture {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.root) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "remove symlink model fixture {}: {error}",
                    self.root.display()
                );
            }
        }
    }
}

fn valid_manifest() -> LocalModelManifest {
    let preset = LocalEmbeddingPreset::default();
    LocalModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        dimensions: preset.dimensions(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        downloaded_at_epoch: 1,
        source_url: Some(preset.source_url()),
        files: vec![LocalModelFile {
            path: "model.onnx".to_string(),
            sha256: "a".repeat(64),
            source_sha256: None,
            bytes: 1,
        }],
        symlinks: vec![],
    }
}

fn equivalent_logical_layout_manifests() -> (LocalModelManifest, LocalModelManifest) {
    let preset = LocalEmbeddingPreset::default();
    let revision = "a".repeat(40);
    let repo_dir = preset.cache_repo_dir();
    let ref_file = LocalModelFile {
        path: format!("{repo_dir}/refs/main"),
        sha256: "f".repeat(64),
        source_sha256: None,
        bytes: revision.len() as u64,
    };
    let mut symlink_files = vec![ref_file.clone()];
    let mut symlinks = Vec::new();
    let mut regular_files = vec![ref_file];
    for (index, runtime_file) in preset.required_runtime_files().enumerate() {
        let sha256 = format!("{:064x}", index + 1);
        let bytes = 100 + index as u64;
        let snapshot_path = format!("{repo_dir}/snapshots/{revision}/{runtime_file}");
        let blob_path = format!("{repo_dir}/blobs/fixture-{index}");
        symlink_files.push(LocalModelFile {
            path: blob_path.clone(),
            sha256: sha256.clone(),
            source_sha256: None,
            bytes,
        });
        symlinks.push(LocalModelSymlink {
            path: snapshot_path.clone(),
            link_target: format!("../../blobs/fixture-{index}"),
            resolved_path: blob_path,
        });
        regular_files.push(LocalModelFile {
            path: snapshot_path,
            sha256,
            source_sha256: None,
            bytes,
        });
    }
    symlink_files.sort_by(|left, right| left.path.cmp(&right.path));
    symlinks.sort_by(|left, right| left.path.cmp(&right.path));
    regular_files.sort_by(|left, right| left.path.cmp(&right.path));
    let base = LocalModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        dimensions: preset.dimensions(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        downloaded_at_epoch: 1,
        source_url: Some(preset.source_url()),
        files: symlink_files,
        symlinks,
    };
    let mut regular = base.clone();
    regular.downloaded_at_epoch = 2;
    regular.files = regular_files;
    regular.symlinks.clear();
    (base, regular)
}

#[test]
fn content_digest_is_independent_of_hf_symlink_or_regular_snapshot_layout() -> Result<()> {
    let (symlink_layout, regular_layout) = equivalent_logical_layout_manifests();

    assert_eq!(
        model_content_sha256(&symlink_layout)?,
        model_content_sha256(&regular_layout)?
    );
    Ok(())
}

#[test]
fn content_digest_changes_when_any_logical_runtime_artifact_changes() -> Result<()> {
    let (baseline, mut changed) = equivalent_logical_layout_manifests();
    let baseline_digest = model_content_sha256(&baseline)?;
    let config = changed
        .files
        .iter_mut()
        .find(|file| file.path.ends_with("/config.json"))
        .context("regular logical fixture should contain config.json")?;
    config.sha256 = "e".repeat(64);

    assert_ne!(baseline_digest, model_content_sha256(&changed)?);
    Ok(())
}

#[test]
fn manifest_header_rejects_wrong_upstream_model() {
    let mut manifest = valid_manifest();
    manifest.upstream_model = "other/model".to_string();
    assert!(verify_manifest_header(&manifest, Some(LocalEmbeddingPreset::default())).is_err());
}

#[test]
fn manifest_header_requires_canonical_source_url() {
    let mut manifest = valid_manifest();
    manifest.source_url = None;
    assert!(verify_manifest_header(&manifest, Some(LocalEmbeddingPreset::default())).is_err());
}

#[test]
fn manifest_header_rejects_unlisted_symlink_target() {
    let mut manifest = valid_manifest();
    manifest.symlinks.push(LocalModelSymlink {
        path: "snapshot/model.onnx".to_string(),
        link_target: "../blobs/model".to_string(),
        resolved_path: "blobs/model".to_string(),
    });
    let error = verify_manifest_header(&manifest, None).unwrap_err();
    assert!(error.to_string().contains("unlisted file"));
}

#[test]
#[cfg(unix)]
fn active_snapshot_symlinks_are_manifested_and_verified() -> Result<()> {
    let fixture = SymlinkModelFixture::new()?;
    let manifest = fixture.publish_manifest()?;

    let verified = read_verified_manifest(&fixture.install_dir, Some(fixture.preset))?;

    assert_eq!(manifest.files.len(), 6);
    assert_eq!(manifest.symlinks.len(), 5);
    assert_eq!(verified.manifest, manifest);
    assert_eq!(verified.artifact_sha256.len(), 64);
    Ok(())
}

#[test]
#[cfg(unix)]
fn cached_verification_rejects_symlink_repoint() -> Result<()> {
    use std::os::unix::fs::symlink;

    let fixture = SymlinkModelFixture::new()?;
    fixture.publish_manifest()?;
    read_verified_manifest(&fixture.install_dir, Some(fixture.preset))?;
    let pointer = fixture.snapshot_path("config.json");
    let replacement = fixture
        .install_dir
        .join(fixture.preset.cache_repo_dir())
        .join("blobs/repointed");
    std::fs::write(&replacement, b"replacement")?;
    std::fs::remove_file(&pointer)?;
    symlink("../../blobs/repointed", &pointer)?;

    let error = read_verified_manifest(&fixture.install_dir, Some(fixture.preset)).unwrap_err();

    assert!(error.to_string().contains("changed target"));
    Ok(())
}

#[test]
#[cfg(unix)]
fn collection_rejects_absolute_and_broken_symlink_targets() -> Result<()> {
    use std::os::unix::fs::symlink;

    for target in ["/tmp/remem-model-escape", "../../blobs/missing"] {
        let fixture = SymlinkModelFixture::new()?;
        let pointer = fixture.snapshot_path("config.json");
        std::fs::remove_file(&pointer)?;
        symlink(target, &pointer)?;

        let error = collect_model_artifacts(&fixture.install_dir, fixture.preset).unwrap_err();

        assert!(
            error
                .to_string()
                .contains(if Path::new(target).is_absolute() {
                    "absolute target"
                } else {
                    "stat symlink target"
                }),
            "{error}"
        );
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn collection_rejects_regular_file_through_intermediate_symlink() -> Result<()> {
    use std::os::unix::fs::symlink;

    let fixture = SymlinkModelFixture::new()?;
    let repo_dir = fixture.install_dir.join(fixture.preset.cache_repo_dir());
    let refs_dir = repo_dir.join("refs");
    let outside_refs = fixture.root.join("outside-refs");
    std::fs::rename(&refs_dir, &outside_refs)?;
    symlink(&outside_refs, &refs_dir)?;

    let error = collect_model_artifacts(&fixture.install_dir, fixture.preset).unwrap_err();

    assert!(
        error.to_string().contains("canonical")
            || error.to_string().contains("escapes verified install"),
        "{error:#}"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn verification_rejects_regular_file_through_intermediate_symlink() -> Result<()> {
    use std::os::unix::fs::symlink;

    let fixture = SymlinkModelFixture::new()?;
    fixture.publish_manifest()?;
    let repo_dir = fixture.install_dir.join(fixture.preset.cache_repo_dir());
    let refs_dir = repo_dir.join("refs");
    let outside_refs = fixture.root.join("outside-refs");
    std::fs::rename(&refs_dir, &outside_refs)?;
    symlink(&outside_refs, &refs_dir)?;

    let error = read_verified_manifest(&fixture.install_dir, Some(fixture.preset)).unwrap_err();

    assert!(
        error.to_string().contains("canonical")
            || error.to_string().contains("escapes verified install"),
        "{error:#}"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn manifest_publish_rejects_existing_symlink_without_touching_target() -> Result<()> {
    use std::os::unix::fs::symlink;

    let fixture = SymlinkModelFixture::new()?;
    let manifest = fixture.publish_manifest()?;
    let manifest_path = fixture.install_dir.join(MANIFEST_FILE);
    let outside_target = fixture.root.join("outside-manifest-target");
    std::fs::write(&outside_target, b"outside-sentinel")?;
    std::fs::remove_file(&manifest_path)?;
    symlink(&outside_target, &manifest_path)?;

    let error = write_manifest(&fixture.install_dir, &manifest).unwrap_err();

    assert!(error.to_string().contains("symlink"), "{error:#}");
    assert_eq!(std::fs::read(&outside_target)?, b"outside-sentinel");
    Ok(())
}

#[test]
#[cfg(unix)]
fn cached_verification_rehashes_same_size_target_corruption() -> Result<()> {
    let fixture = SymlinkModelFixture::new()?;
    let manifest = fixture.publish_manifest()?;
    read_verified_manifest(&fixture.install_dir, Some(fixture.preset))?;
    let target = manifest
        .symlinks
        .iter()
        .find(|symlink| symlink.path.ends_with("/config.json"))
        .context("config symlink should be present")?;
    let target_path = fixture.install_dir.join(&target.resolved_path);
    let original = std::fs::read(&target_path)?;
    std::fs::write(&target_path, vec![b'x'; original.len()])?;

    let error = read_verified_manifest(&fixture.install_dir, Some(fixture.preset)).unwrap_err();

    assert!(error.to_string().contains("checksum mismatch"), "{error}");
    Ok(())
}

#[test]
#[cfg(unix)]
fn active_revision_change_fails_closed() -> Result<()> {
    let fixture = SymlinkModelFixture::new()?;
    fixture.publish_manifest()?;
    read_verified_manifest(&fixture.install_dir, Some(fixture.preset))?;
    let ref_path = fixture
        .install_dir
        .join(fixture.preset.cache_repo_dir())
        .join("refs/main");
    std::fs::write(ref_path, "b".repeat(40))?;

    let error = read_verified_manifest(&fixture.install_dir, Some(fixture.preset)).unwrap_err();

    assert!(
        error.to_string().contains("missing required runtime file"),
        "{error}"
    );
    Ok(())
}

#[test]
#[cfg(unix)]
fn released_schema_v1_install_upgrades_offline_to_schema_v2() -> Result<()> {
    let fixture = SymlinkModelFixture::new()?;
    fixture.publish_legacy_manifest(None)?;
    let legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.install_dir.join(MANIFEST_FILE))?)?;
    assert_eq!(legacy["schema_version"], serde_json::json!(1));
    assert!(
        legacy["files"]
            .as_array()
            .context("legacy fixture should declare files")?
            .iter()
            .any(|file| file["path"]
                .as_str()
                .is_some_and(|path| path.contains("/snapshots/"))),
        "released schema-v1 manifests recorded followed snapshot symlinks as files"
    );

    let verified = read_verified_manifest_compatible(&fixture.install_dir, Some(fixture.preset))?;

    assert_eq!(verified.manifest.schema_version, MANIFEST_SCHEMA_VERSION);
    assert_eq!(verified.manifest.files.len(), 6);
    assert_eq!(verified.manifest.symlinks.len(), 5);
    assert!(fixture.install_dir.join(MODEL_DOWNLOAD_LOCK_FILE).is_file());
    assert!(fixture.install_dir.join(MODEL_STATE_LOCK_FILE).is_file());
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.install_dir.join(MANIFEST_FILE))?)?;
    assert_eq!(
        persisted["schema_version"],
        serde_json::json!(MANIFEST_SCHEMA_VERSION)
    );
    assert_eq!(persisted["symlinks"].as_array().map(Vec::len), Some(5));
    Ok(())
}

#[test]
#[cfg(unix)]
fn schema_v1_upgrade_rejects_runtime_blob_not_bound_by_legacy_manifest() -> Result<()> {
    let fixture = SymlinkModelFixture::new()?;
    fixture.publish_legacy_manifest(Some("blobs/fixture-blob-1"))?;

    let error =
        read_verified_manifest_compatible(&fixture.install_dir, Some(fixture.preset)).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("was not bound by schema-v1 manifest"),
        "{error:#}"
    );
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.install_dir.join(MANIFEST_FILE))?)?;
    assert_eq!(persisted["schema_version"], serde_json::json!(1));
    Ok(())
}

#[test]
#[cfg(unix)]
fn bge_missing_external_data_fails_manifest_verification_before_runtime() -> Result<()> {
    let fixture = SymlinkModelFixture::new_for_preset(LocalEmbeddingPreset::BgeM3)?;
    let manifest = fixture.publish_manifest()?;
    let external_data = manifest
        .symlinks
        .iter()
        .find(|symlink| symlink.path.ends_with("/onnx/model.onnx_data"))
        .context("BGE fixture should include model.onnx_data")?;
    std::fs::remove_file(fixture.install_dir.join(&external_data.resolved_path))?;

    let error = read_verified_manifest(&fixture.install_dir, Some(fixture.preset)).unwrap_err();

    assert!(
        error.to_string().contains("model.onnx_data")
            || error.to_string().contains(&external_data.resolved_path),
        "{error:#}"
    );
    Ok(())
}
