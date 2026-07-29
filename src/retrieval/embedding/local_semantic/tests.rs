use super::*;

mod model_state_pin;
mod verification_cache;

#[test]
fn hf_cache_blob_source_sha_is_verified() -> Result<()> {
    let sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let verified = source_sha256_from_hf_blob_path(&format!("models--demo/blobs/{sha}"), sha)?;

    assert_eq!(verified.as_deref(), Some(sha));
    Ok(())
}

#[test]
fn hf_cache_blob_source_sha_mismatch_fails() {
    let source = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let actual = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

    let error = source_sha256_from_hf_blob_path(&format!("models--demo/blobs/{source}"), actual)
        .unwrap_err();

    assert!(error.to_string().contains("source checksum mismatch"));
}

#[test]
#[cfg(feature = "local-onnx")]
fn verified_manifest_cache_avoids_rehashing_unchanged_model_files() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let model_root = std::env::temp_dir().join(format!(
        "remem-manifest-cache-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    install_test_model(&model_root)?;
    let config = EmbeddingConfig {
        model_dir: Some(model_root.display().to_string()),
        ..EmbeddingConfig::default()
    };
    let hash_counter = ModelFileHashCounter::start(&model_root)?;

    let first = installed_model_profile(&config)?;
    let first_hash_count = hash_counter.count()?;
    let second = installed_model_profile(&config)?;
    let second_hash_count = hash_counter.count()?;

    assert_eq!(first, second);
    assert!(first_hash_count > 0);
    assert_eq!(second_hash_count, first_hash_count);
    std::fs::remove_dir_all(&model_root)
        .with_context(|| format!("remove test model root {}", model_root.display()))?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn artifact_digest_qualifies_persisted_model_identity() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let model_root = std::env::temp_dir().join(format!(
        "remem-artifact-profile-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    install_test_model(&model_root)?;
    let config = EmbeddingConfig {
        model_dir: Some(model_root.display().to_string()),
        ..EmbeddingConfig::default()
    };
    let first = installed_model_profile(&config)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    let runtime_file = test_model_runtime_file(&model_root, "config.json");
    std::fs::write(&runtime_file, b"different-test-model-config")?;
    let manifest_path = install_dir.join(MANIFEST_FILE);
    let mut manifest: LocalModelManifest = serde_json::from_slice(&std::fs::read(&manifest_path)?)?;
    let (files, symlinks) = collect_model_artifacts(&install_dir, preset)?;
    manifest.files = files;
    manifest.symlinks = symlinks;
    write_manifest(&install_dir, &manifest)?;

    let second = installed_model_profile(&config)?;

    assert_ne!(first.artifact_sha256, second.artifact_sha256);
    assert_ne!(first.model, second.model);
    assert_eq!(
        second.model,
        format!(
            "{}@sha256:{}",
            DEFAULT_LOCAL_SEMANTIC_MODEL, second.artifact_sha256
        )
    );
    std::fs::remove_dir_all(&model_root)
        .with_context(|| format!("remove test model root {}", model_root.display()))?;
    Ok(())
}

#[test]
fn model_file_hash_count_is_isolated_by_canonical_root_under_concurrency() -> Result<()> {
    let root = std::env::temp_dir().join(format!(
        "remem-model-hash-counter-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let root_a = root.join("a");
    let root_b = root.join("b");
    std::fs::create_dir_all(&root_a)?;
    std::fs::create_dir_all(&root_b)?;
    let file_a = root_a.join("model.bin");
    let file_b = root_b.join("model.bin");
    std::fs::write(&file_a, b"model-a")?;
    std::fs::write(&file_b, b"model-b")?;
    let counter_a = ModelFileHashCounter::start(&root_a)?;
    let counter_b = ModelFileHashCounter::start(&root_b)?;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let mut threads = Vec::new();

    for thread_index in 0..8 {
        let barrier = std::sync::Arc::clone(&barrier);
        let path = if thread_index % 2 == 0 {
            file_a.clone()
        } else {
            file_b.clone()
        };
        threads.push(std::thread::spawn(move || -> Result<()> {
            barrier.wait();
            for _ in 0..25 {
                sha256_file(&path)?;
            }
            Ok(())
        }));
    }
    for thread in threads {
        thread
            .join()
            .map_err(|_| anyhow::anyhow!("model hash counter test thread panicked"))??;
    }

    assert_eq!(counter_a.count()?, 100);
    assert_eq!(counter_b.count()?, 100);
    std::fs::remove_dir_all(&root)
        .with_context(|| format!("remove hash counter root {}", root.display()))?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn unapproved_default_download_is_rejected_before_runtime_probe_or_manifest_publish() -> Result<()>
{
    let root = std::env::temp_dir().join(format!(
        "remem-unapproved-download-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    install_untrusted_test_model(&root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = root.join(preset.model_id());
    let manifest_path = install_dir.join(MANIFEST_FILE);
    std::fs::remove_file(&manifest_path)?;
    let probe_calls = std::cell::Cell::new(0);

    let error = download::prepare_downloaded_model_with(preset, &install_dir, 1, |_, _| {
        probe_calls.set(probe_calls.get() + 1);
        Ok(())
    })
    .unwrap_err();

    assert!(
        error.to_string().contains("unapproved content"),
        "{error:#}"
    );
    assert!(
        error
            .to_string()
            .contains("No downloaded model bytes were executed"),
        "{error:#}"
    );
    assert_eq!(probe_calls.get(), 0);
    assert!(!manifest_path.exists());
    std::fs::remove_dir_all(&root)
        .with_context(|| format!("remove download pin test root {}", root.display()))?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn explicit_bge_candidate_is_fully_verified_before_probe_without_early_publish() -> Result<()> {
    let root = std::env::temp_dir().join(format!(
        "remem-bge-download-order-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let preset = LocalEmbeddingPreset::BgeM3;
    test_support::install_test_model_for_preset(&root, preset)?;
    let install_dir = root.join(preset.model_id());
    let manifest_path = install_dir.join(MANIFEST_FILE);
    std::fs::remove_file(&manifest_path)?;
    let hash_counter = ModelFileHashCounter::start(&install_dir)?;
    let probe_calls = std::cell::Cell::new(0);

    let prepared =
        download::prepare_downloaded_model_with(preset, &install_dir, 1, |manifest, digest| {
            assert!(!manifest_path.exists());
            assert_eq!(manifest::model_content_sha256(manifest)?, digest);
            assert!(
                hash_counter.count()? >= manifest.files.len() * 2,
                "collection and unpublished-manifest verification must both finish before the probe"
            );
            probe_calls.set(probe_calls.get() + 1);
            Ok(())
        })?;

    assert_eq!(probe_calls.get(), 1);
    assert!(!manifest_path.exists());
    assert_eq!(
        prepared.artifact_sha256,
        manifest::model_content_sha256(&prepared.manifest)?
    );
    std::fs::remove_dir_all(&root)
        .with_context(|| format!("remove BGE download order test root {}", root.display()))?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn download_rejects_untrusted_hf_endpoint_before_loader() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let _endpoint = TestEnvRestore::capture(HUGGING_FACE_ENDPOINT_ENV);
    let _hf_home = TestEnvRestore::capture("HF_HOME");
    unsafe { std::env::remove_var("HF_HOME") };
    let loader_calls = std::cell::Cell::new(0);

    for endpoint in [
        "",
        "http://huggingface.co",
        "https://mirror.invalid",
        "https://huggingface.co//",
    ] {
        unsafe { std::env::set_var(HUGGING_FACE_ENDPOINT_ENV, endpoint) };
        let error = download::with_official_hugging_face_endpoint(|| {
            loader_calls.set(loader_calls.get() + 1);
            Ok(())
        })
        .unwrap_err();
        assert!(
            error.to_string().contains(HUGGING_FACE_ENDPOINT_ENV),
            "{error:#}"
        );
    }

    assert_eq!(loader_calls.get(), 0);
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn download_accepts_only_unset_or_official_hf_endpoint() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let _endpoint = TestEnvRestore::capture(HUGGING_FACE_ENDPOINT_ENV);
    let _hf_home = TestEnvRestore::capture("HF_HOME");
    unsafe { std::env::remove_var("HF_HOME") };
    let loader_calls = std::cell::Cell::new(0);

    for endpoint in [
        None,
        Some(HUGGING_FACE_BASE_URL),
        Some("https://huggingface.co/"),
    ] {
        match endpoint {
            Some(endpoint) => unsafe { std::env::set_var(HUGGING_FACE_ENDPOINT_ENV, endpoint) },
            None => unsafe { std::env::remove_var(HUGGING_FACE_ENDPOINT_ENV) },
        }
        download::with_official_hugging_face_endpoint(|| {
            loader_calls.set(loader_calls.get() + 1);
            Ok(())
        })?;
    }

    assert_eq!(loader_calls.get(), 3);
    Ok(())
}

#[test]
#[cfg(all(feature = "local-onnx", unix))]
fn download_rejects_non_unicode_hf_endpoint_before_loader() -> Result<()> {
    use std::os::unix::ffi::OsStringExt;

    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let _endpoint = TestEnvRestore::capture(HUGGING_FACE_ENDPOINT_ENV);
    let _hf_home = TestEnvRestore::capture("HF_HOME");
    unsafe {
        std::env::remove_var("HF_HOME");
        std::env::set_var(
            HUGGING_FACE_ENDPOINT_ENV,
            std::ffi::OsString::from_vec(vec![0xff]),
        );
    }
    let loader_calls = std::cell::Cell::new(0);

    let error = download::with_official_hugging_face_endpoint(|| {
        loader_calls.set(loader_calls.get() + 1);
        Ok(())
    })
    .unwrap_err();

    assert!(error.to_string().contains("non-Unicode"), "{error:#}");
    assert_eq!(loader_calls.get(), 0);
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn download_ignores_unrelated_hf_home_and_uses_fresh_staging() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let _endpoint = TestEnvRestore::capture(HUGGING_FACE_ENDPOINT_ENV);
    let _hf_home = TestEnvRestore::capture("HF_HOME");
    unsafe {
        std::env::remove_var(HUGGING_FACE_ENDPOINT_ENV);
        std::env::set_var("HF_HOME", "/tmp/unrelated-hugging-face-cache");
    }
    let root = std::env::temp_dir().join(format!(
        "remem-download-staging-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let install_dir = root.join("model");
    test_support::create_test_owner_only_install(&root, &install_dir)?;
    let loader_calls = std::cell::Cell::new(0);

    let staging = download::materialize_hugging_face_artifacts_with(&install_dir, |staging_dir| {
        loader_calls.set(loader_calls.get() + 1);
        assert_ne!(staging_dir, install_dir);
        assert_eq!(
            staging_dir.parent(),
            Some(std::fs::canonicalize(&install_dir)?.as_path())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(staging_dir)?.permissions().mode() & 0o777,
                0o700
            );
        }
        Ok(())
    })?;

    assert_eq!(loader_calls.get(), 1);
    staging.cleanup()?;
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn failed_staged_download_cleans_candidate_tree() -> Result<()> {
    let _guard = crate::runtime_config::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let _endpoint = TestEnvRestore::capture(HUGGING_FACE_ENDPOINT_ENV);
    unsafe { std::env::remove_var(HUGGING_FACE_ENDPOINT_ENV) };
    let root = test_download_root("failed-staging");
    let install_dir = root.join("model");
    test_support::create_test_owner_only_install(&root, &install_dir)?;

    let error = download::materialize_hugging_face_artifacts_with(&install_dir, |staging_dir| {
        std::fs::write(staging_dir.join("partial"), b"partial download")?;
        bail!("injected staged download failure")
    })
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("injected staged download failure"),
        "{error:#}"
    );
    assert!(
        std::fs::read_dir(&install_dir)?.next().is_none(),
        "failed staging must be removed"
    );
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn prepared_candidate_publishes_into_fresh_install_transactionally() -> Result<()> {
    let staging_root = test_download_root("publish-source");
    install_test_model(&staging_root)?;
    let preset = LocalEmbeddingPreset::default();
    let staging_dir = staging_root.join(preset.model_id());
    std::fs::remove_file(staging_dir.join(MANIFEST_FILE))?;
    let prepared = download::prepare_downloaded_model_with(preset, &staging_dir, 2, |_, _| Ok(()))?;
    let final_root = test_download_root("publish-target");
    let install_dir = final_root.join(preset.model_id());
    test_support::create_test_owner_only_install(&final_root, &install_dir)?;
    drop(manifest::open_or_create_model_lock(
        &install_dir,
        MODEL_DOWNLOAD_LOCK_FILE,
    )?);

    let imported = download::import_immutable_candidate(
        &staging_dir,
        &install_dir,
        preset,
        &prepared.manifest,
    )?;
    let expected_sha256 = prepared.artifact_sha256.clone();
    let published = download::activate_candidate_manifest(
        &install_dir,
        preset,
        prepared.manifest,
        prepared.artifact_sha256,
        imported,
    )?;

    assert_eq!(published.artifact_sha256, expected_sha256);
    let reread = read_verified_manifest_unlocked(&install_dir, Some(preset))?;
    assert_eq!(reread.artifact_sha256, expected_sha256);
    std::fs::remove_dir_all(&staging_root)?;
    std::fs::remove_dir_all(&final_root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn failed_candidate_activation_restores_previous_profile_and_manifest() -> Result<()> {
    let model_root = test_download_root("activation-rollback");
    install_test_model(&model_root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    let config = EmbeddingConfig {
        model_dir: Some(model_root.display().to_string()),
        ..EmbeddingConfig::default()
    };
    let before_profile = installed_model_profile(&config)?;
    let before_manifest = std::fs::read(install_dir.join(MANIFEST_FILE))?;
    let previous = read_verified_manifest_unlocked(&install_dir, Some(preset))?;
    let failed_revision = "b".repeat(40);
    let verification =
        manifest::verify_unpublished_candidate(&install_dir, &previous.manifest, Some(preset))?;

    let error = download::activate_candidate_manifest(
        &install_dir,
        preset,
        previous.manifest,
        previous.artifact_sha256,
        download::ImportedLocalModel {
            revision: failed_revision,
            verification,
        },
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("checksum")
            || error.to_string().contains("missing required runtime file"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read_to_string(install_dir.join(preset.cache_repo_dir()).join("refs/main"))?,
        "a".repeat(40)
    );
    assert_eq!(
        std::fs::read(install_dir.join(MANIFEST_FILE))?,
        before_manifest
    );
    assert_eq!(installed_model_profile(&config)?, before_profile);
    std::fs::remove_dir_all(&model_root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn interrupted_prepared_activation_recovers_previous_state_on_next_read() -> Result<()> {
    let model_root = test_download_root("activation-prepared-crash");
    install_test_model(&model_root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    let before_manifest = std::fs::read(install_dir.join(MANIFEST_FILE))?;
    let previous = read_verified_manifest_unlocked(&install_dir, Some(preset))?;

    let transaction = manifest::ActiveRevisionTransaction::begin(
        &install_dir,
        preset,
        &"b".repeat(40),
        &previous.manifest,
    )?;
    std::mem::forget(transaction);

    let recovered = manifest::read_verified_manifest(&install_dir, Some(preset))?;

    assert_eq!(recovered.artifact_sha256, previous.artifact_sha256);
    assert_eq!(
        std::fs::read_to_string(install_dir.join(preset.cache_repo_dir()).join("refs/main"))?,
        "a".repeat(40)
    );
    assert_eq!(
        std::fs::read(install_dir.join(MANIFEST_FILE))?,
        before_manifest
    );
    assert!(!install_dir.join(".remem-model-activation.json").exists());
    std::fs::remove_dir_all(&model_root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn interrupted_committing_activation_finishes_candidate_on_next_read() -> Result<()> {
    let model_root = test_download_root("activation-committing-crash");
    install_test_model(&model_root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    let revision = "b".repeat(40);
    let candidate = candidate_manifest_for_revision(&install_dir, preset, &revision)?;
    let expected_sha256 = manifest::model_content_sha256(&candidate)?;

    let mut transaction =
        manifest::ActiveRevisionTransaction::begin(&install_dir, preset, &revision, &candidate)?;
    let verified = manifest::verify_unpublished_manifest(&install_dir, &candidate, Some(preset))?;
    assert_eq!(verified, expected_sha256);
    transaction.mark_committing()?;
    std::mem::forget(transaction);

    let recovered = manifest::read_verified_manifest(&install_dir, Some(preset))?;

    assert_eq!(recovered.artifact_sha256, expected_sha256);
    assert_eq!(recovered.manifest.downloaded_at_epoch, 2);
    assert_eq!(
        std::fs::read_to_string(install_dir.join(preset.cache_repo_dir()).join("refs/main"))?,
        revision
    );
    assert!(!install_dir.join(".remem-model-activation.json").exists());
    std::fs::remove_dir_all(&model_root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn interrupted_committing_activation_rolls_back_missing_candidate_file() -> Result<()> {
    let model_root = test_download_root("activation-missing-candidate");
    install_test_model(&model_root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    let before = read_verified_manifest_unlocked(&install_dir, Some(preset))?;
    let before_manifest = std::fs::read(install_dir.join(MANIFEST_FILE))?;
    let revision = "b".repeat(40);
    let candidate = candidate_manifest_for_revision(&install_dir, preset, &revision)?;

    let mut transaction =
        manifest::ActiveRevisionTransaction::begin(&install_dir, preset, &revision, &candidate)?;
    manifest::verify_unpublished_manifest(&install_dir, &candidate, Some(preset))?;
    transaction.mark_committing()?;
    write_manifest(&install_dir, &candidate)?;
    std::fs::remove_file(
        install_dir
            .join(preset.cache_repo_dir())
            .join("snapshots")
            .join(&revision)
            .join("config.json"),
    )?;
    std::mem::forget(transaction);

    let recovered = manifest::read_verified_manifest(&install_dir, Some(preset))?;

    assert_eq!(recovered.artifact_sha256, before.artifact_sha256);
    assert_eq!(
        std::fs::read_to_string(install_dir.join(preset.cache_repo_dir()).join("refs/main"))?,
        "a".repeat(40)
    );
    assert_eq!(
        std::fs::read(install_dir.join(MANIFEST_FILE))?,
        before_manifest
    );
    assert!(!install_dir.join(".remem-model-activation.json").exists());
    std::fs::remove_dir_all(&model_root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn active_model_reads_do_not_wait_for_download_serialization_lock() -> Result<()> {
    let model_root = test_download_root("nonblocking-redownload");
    install_test_model(&model_root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    let (_, download_lock) =
        manifest::open_or_create_model_lock(&install_dir, MODEL_DOWNLOAD_LOCK_FILE)?;
    fs2::FileExt::lock_exclusive(&download_lock)?;
    let config = EmbeddingConfig {
        model_dir: Some(model_root.display().to_string()),
        ..EmbeddingConfig::default()
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let _ = sender.send(installed_model_profile(&config));
    });

    let received = receiver.recv_timeout(std::time::Duration::from_millis(500));
    fs2::FileExt::unlock(&download_lock)?;
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("active model reader panicked"))?;
    let profile =
        received.context("active model read blocked on the long download serialization lock")??;

    assert!(profile.model.starts_with(preset.model_id()));
    drop(download_lock);
    std::fs::remove_dir_all(&model_root)?;
    Ok(())
}

#[test]
#[cfg(feature = "local-onnx")]
fn first_install_unavailable_check_does_not_wait_for_download_serialization_lock() -> Result<()> {
    let model_root = test_download_root("nonblocking-first-download");
    let preset = LocalEmbeddingPreset::default();
    let install_dir = model_root.join(preset.model_id());
    test_support::create_test_owner_only_install(&model_root, &install_dir)?;
    let (_, download_lock) =
        manifest::open_or_create_model_lock(&install_dir, MODEL_DOWNLOAD_LOCK_FILE)?;
    let state_lock = manifest::open_or_create_model_lock(&install_dir, MODEL_STATE_LOCK_FILE)?;
    fs2::FileExt::lock_exclusive(&download_lock)?;
    let config = EmbeddingConfig {
        model_dir: Some(model_root.display().to_string()),
        ..EmbeddingConfig::default()
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let _ = sender.send(auto_installed_model_profile(&config));
    });

    let received = receiver.recv_timeout(std::time::Duration::from_millis(500));
    fs2::FileExt::unlock(&download_lock)?;
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("first-install reader panicked"))?;
    let error = received
        .context("first-install availability check blocked on the download lock")?
        .unwrap_err();

    assert!(is_model_unavailable_error(&error), "{error:#}");
    drop(state_lock);
    drop(download_lock);
    std::fs::remove_dir_all(&model_root)?;
    Ok(())
}

#[test]
#[cfg(all(feature = "local-onnx", unix))]
fn malicious_preseeded_pinned_ref_symlink_is_rejected_without_touching_target() -> Result<()> {
    use std::os::unix::fs::symlink;

    let final_root = test_download_root("preseed-final");
    install_test_model(&final_root)?;
    let preset = LocalEmbeddingPreset::default();
    let install_dir = final_root.join(preset.model_id());
    let before = read_verified_manifest_unlocked(&install_dir, Some(preset))?;
    let staging_root = test_download_root("preseed-staging");
    install_test_model(&staging_root)?;
    let staging_dir = staging_root.join(preset.model_id());
    std::fs::remove_file(staging_dir.join(MANIFEST_FILE))?;
    let prepared = download::prepare_downloaded_model_with(preset, &staging_dir, 2, |_, _| Ok(()))?;
    let victim = final_root.join("victim");
    std::fs::write(&victim, b"outside-sentinel")?;
    let pinned_ref = install_dir
        .join(preset.cache_repo_dir())
        .join("refs")
        .join("a".repeat(40));
    symlink(&victim, &pinned_ref)?;

    let error = download::import_immutable_candidate(
        &staging_dir,
        &install_dir,
        preset,
        &prepared.manifest,
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("not a regular file"),
        "{error:#}"
    );
    assert_eq!(std::fs::read(&victim)?, b"outside-sentinel");
    let after = read_verified_manifest_unlocked(&install_dir, Some(preset))?;
    assert_eq!(after.artifact_sha256, before.artifact_sha256);
    std::fs::remove_file(&pinned_ref)?;
    std::fs::remove_dir_all(&staging_root)?;
    std::fs::remove_dir_all(&final_root)?;
    Ok(())
}

#[cfg(feature = "local-onnx")]
fn candidate_manifest_for_revision(
    install_dir: &Path,
    preset: LocalEmbeddingPreset,
    revision: &str,
) -> Result<LocalModelManifest> {
    let repo_dir = install_dir.join(preset.cache_repo_dir());
    let previous_revision = "a".repeat(40);
    for runtime_file in preset.required_runtime_files() {
        let source = repo_dir
            .join("snapshots")
            .join(&previous_revision)
            .join(runtime_file);
        let destination = repo_dir.join("snapshots").join(revision).join(runtime_file);
        std::fs::create_dir_all(
            destination
                .parent()
                .context("candidate runtime file should have a parent")?,
        )?;
        std::fs::copy(&source, &destination)?;
    }
    let active_ref = repo_dir.join("refs/main");
    std::fs::write(&active_ref, revision)?;
    let collected = collect_model_artifacts(install_dir, preset);
    std::fs::write(&active_ref, &previous_revision)?;
    let (files, symlinks) = collected?;
    Ok(LocalModelManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        preset: preset.label().to_string(),
        model_id: preset.model_id().to_string(),
        upstream_model: preset.upstream_model().to_string(),
        dimensions: preset.dimensions(),
        runtime: FASTEMBED_RUNTIME.to_string(),
        source_url: Some(preset.source_url()),
        downloaded_at_epoch: 2,
        files,
        symlinks,
    })
}

#[cfg(feature = "local-onnx")]
fn test_download_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

#[cfg(feature = "local-onnx")]
struct TestEnvRestore {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

#[cfg(feature = "local-onnx")]
impl TestEnvRestore {
    fn capture(key: &'static str) -> Self {
        Self {
            key,
            previous: std::env::var_os(key),
        }
    }
}

#[cfg(feature = "local-onnx")]
impl Drop for TestEnvRestore {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}
