use anyhow::Context;

use super::tests::with_clean_env;
use super::*;

#[test]
fn local_only_query_uses_resolved_feature_hash_fallback_without_api_key() -> Result<()> {
    with_clean_env(|| {
        unsafe {
            std::env::set_var(ENV_PROVIDER, "api");
            std::env::set_var(ENV_FALLBACK, "feature-hash");
        }

        let embedding = embed_query_local_only_if_enabled("local fallback context")?
            .context("resolved local fallback should remain available")?;

        assert_eq!(embedding.model(), FEATURE_HASH_EMBEDDING_MODEL);
        assert_eq!(embedding.dimensions(), FEATURE_HASH_EMBEDDING_DIMENSIONS);
        Ok(())
    })
}

#[test]
fn local_only_query_skips_unavailable_api_without_local_fallback() -> Result<()> {
    with_clean_env(|| {
        unsafe {
            std::env::set_var(ENV_PROVIDER, "api");
        }

        assert!(embed_query_local_only_if_enabled("lexical context")?.is_none());

        unsafe {
            std::env::set_var(ENV_FALLBACK, "off");
        }
        assert!(embed_query_local_only_if_enabled("lexical context")?.is_none());
        Ok(())
    })
}

#[test]
fn local_only_profile_fingerprint_tracks_effective_vector_policy() -> Result<()> {
    with_clean_env(|| {
        unsafe {
            std::env::set_var(ENV_PROVIDER, "feature-hash");
        }
        let feature_hash = local_only_embedding_profile_fingerprint();

        unsafe {
            std::env::set_var(ENV_PROVIDER, "off");
        }
        let off = local_only_embedding_profile_fingerprint();

        unsafe {
            std::env::set_var(ENV_PROVIDER, "api");
            std::env::remove_var(ENV_FALLBACK);
        }
        let skipped_remote = local_only_embedding_profile_fingerprint();

        assert_eq!(feature_hash.len(), 64);
        assert_eq!(off.len(), 64);
        assert_eq!(skipped_remote.len(), 64);
        assert_ne!(feature_hash, off);
        assert_ne!(feature_hash, skipped_remote);
        assert_ne!(off, skipped_remote);
        Ok(())
    })
}
