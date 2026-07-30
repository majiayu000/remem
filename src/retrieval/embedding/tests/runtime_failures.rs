use super::*;

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_race_to_typed_local_unavailable_caches_feature_hash() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        let _failure = local_semantic::fail_next_test_model_embed_unavailable(
            model_root.path(),
            "synthetic local model disappeared after provider resolution",
        )?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };
        let mut cache = EmbeddingFallbackCache::default();

        let embedding = embed_query_with_fallback_cache("provider resolution race", &mut cache)?;
        let target = cache
            .call_failure_fallback_target()
            .context("race fallback should cache a target")?;

        assert_eq!(embedding.model(), FEATURE_HASH_EMBEDDING_MODEL);
        assert_eq!(target.model, FEATURE_HASH_EMBEDDING_MODEL);
        assert_eq!(target.dimensions, FEATURE_HASH_EMBEDDING_DIMENSIONS);
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_race_does_not_swallow_generic_inference_error() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        let _failure = local_semantic::fail_next_test_model_embed_generic(
            model_root.path(),
            "synthetic generic local inference failure",
        )?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };
        let mut cache = EmbeddingFallbackCache::default();

        let error =
            embed_query_with_fallback_cache("generic inference race", &mut cache).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("synthetic generic local inference failure"),
            "{error:#}"
        );
        assert!(cache.call_failure_fallback_target().is_none());
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_rejects_unpinned_artifact_before_runtime_constructor() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        local_semantic::install_untrusted_test_model(model_root.path())?;
        let _failure = local_semantic::fail_test_model_runtime_readiness(
            model_root.path(),
            "constructor must not run for an unpinned Auto artifact",
        )?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };

        let status = embedding_provider_status_without_probe()?;
        let reason = status.degradation_reason.as_deref().unwrap_or_default();

        assert_eq!(status.configured_provider, "auto");
        assert_eq!(status.active_provider, "feature-hash");
        assert!(status.degraded);
        assert!(reason.contains(local_semantic::AUTO_EVALUATED_DEFAULT_ARTIFACT_SHA256));
        assert!(reason.contains("not trusted for Auto"));
        assert!(!reason.contains("constructor must not run"), "{reason}");
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn explicit_local_runtime_constructor_failure_is_unavailable() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        local_semantic::install_untrusted_test_model(model_root.path())?;
        let _failure = local_semantic::fail_test_model_runtime_readiness(
            model_root.path(),
            "synthetic explicit-local constructor failure",
        )?;
        unsafe {
            std::env::set_var(ENV_MODEL_DIR, model_root.path());
            std::env::set_var(ENV_PROVIDER, "local");
        }

        let status = embedding_provider_status_without_probe()?;
        let error = embed_query("constructor readiness probe").unwrap_err();

        assert_eq!(status.configured_provider, "local");
        assert_eq!(status.active_provider, "local");
        assert!(status.degraded);
        assert!(status
            .unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .contains("synthetic explicit-local constructor failure"));
        assert!(is_local_embedding_model_unavailable_error(&error));
        assert!(
            error
                .to_string()
                .contains("synthetic explicit-local constructor failure"),
            "{error:#}"
        );
        Ok(())
    })
}
