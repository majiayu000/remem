use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::*;

#[cfg(feature = "local-onnx")]
mod runtime_failures;

const TEST_API_KEY_ENV: &str = "REMEM_TEST_EMBEDDING_KEY";

const ENV_KEYS: &[&str] = &[
    "REMEM_CONFIG",
    ENV_PROVIDER,
    ENV_PROVIDER_LEGACY,
    ENV_MODEL,
    ENV_MODEL_LEGACY,
    ENV_BASE_URL,
    ENV_BASE_URL_LEGACY,
    ENV_DIMENSIONS,
    ENV_DIMENSIONS_LEGACY,
    ENV_API_KEY,
    ENV_API_KEY_LEGACY,
    ENV_API_KEY_ENV,
    ENV_TIMEOUT_SECS,
    ENV_FALLBACK,
    ENV_MODEL_DIR,
    "HF_HOME",
    "HF_ENDPOINT",
    DEFAULT_API_KEY_ENV,
    TEST_API_KEY_ENV,
];

struct TestModelRoot(PathBuf);

impl TestModelRoot {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "remem-embedding-model-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestModelRoot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "failed to remove test model root {}: {error}",
                    self.0.display()
                );
            }
        }
    }
}

struct CleanEnv {
    saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    _guard: crate::runtime_config::TestEnvGuard,
}

impl CleanEnv {
    fn new() -> Self {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let saved = ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect::<Vec<_>>();
        for key in ENV_KEYS {
            unsafe { std::env::remove_var(key) };
        }
        let isolated_config = std::env::temp_dir().join(format!(
            "remem-embedding-missing-config-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        unsafe { std::env::set_var("REMEM_CONFIG", isolated_config) };
        Self {
            saved,
            _guard: guard,
        }
    }
}

impl Drop for CleanEnv {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn with_clean_env<T>(f: impl FnOnce() -> T) -> T {
    let _env = CleanEnv::new();
    f()
}

#[test]
fn auto_provider_uses_feature_hash_without_key_or_installed_model() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };

        let embedding = embed_query("protect persisted data")?;
        let status = embedding_provider_status()?;

        assert_eq!(embedding.model(), FEATURE_HASH_EMBEDDING_MODEL);
        assert_eq!(embedding.dimensions(), FEATURE_HASH_EMBEDDING_DIMENSIONS);
        assert_eq!(status.configured_provider, "auto");
        assert_eq!(status.active_provider, "feature-hash");
        Ok(())
    })
}

#[test]
#[cfg(unix)]
fn auto_provider_marks_dangling_local_install_symlink_as_degraded() -> Result<()> {
    use std::os::unix::fs::symlink;

    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        std::fs::create_dir_all(model_root.path())?;
        let install_dir = model_root
            .path()
            .join(local_semantic::DEFAULT_LOCAL_SEMANTIC_MODEL);
        symlink(
            model_root.path().join("missing-install-target"),
            &install_dir,
        )?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.active_provider, "feature-hash");
        assert!(status.degraded);
        assert!(
            status
                .degradation_reason
                .as_deref()
                .unwrap_or_default()
                .contains("symlink"),
            "{status:?}"
        );
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_uses_installed_local_model_without_api_key() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.configured_provider, "auto");
        assert_eq!(status.active_provider, "local");
        let active_model_id = status.active_model_id.as_deref().unwrap_or_default();
        assert!(active_model_id.starts_with(&format!(
            "{}@sha256:",
            local_semantic::DEFAULT_LOCAL_SEMANTIC_MODEL
        )));
        assert_eq!(
            active_model_id.len(),
            local_semantic::DEFAULT_LOCAL_SEMANTIC_MODEL.len() + "@sha256:".len() + 64
        );
        assert_eq!(
            status.active_dimensions,
            Some(local_semantic::DEFAULT_LOCAL_SEMANTIC_DIMENSIONS)
        );
        assert!(!status.degraded);
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_upgrades_released_schema_v1_install_without_network() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        local_semantic::install_test_model_v1(model_root.path())?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.active_provider, "local");
        assert!(!status.degraded);
        let manifest_path = model_root
            .path()
            .join(local_semantic::DEFAULT_LOCAL_SEMANTIC_MODEL)
            .join("remem-model-manifest.json");
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
        assert_eq!(manifest["schema_version"], serde_json::json!(2));
        assert!(manifest["symlinks"].is_array());
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_ignores_ambient_openai_key_when_local_model_is_installed() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe {
            std::env::set_var(ENV_MODEL_DIR, model_root.path());
            std::env::set_var(DEFAULT_API_KEY_ENV, "ambient-key");
        }

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.active_provider, "local");
        assert!(status
            .active_model_id
            .as_deref()
            .unwrap_or_default()
            .starts_with(&format!(
                "{}@sha256:",
                local_semantic::DEFAULT_LOCAL_SEMANTIC_MODEL
            )));
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_marks_invalid_local_install_as_degraded_feature_hash() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };
        let ready = embedding_provider_status_without_probe()?;
        assert_eq!(ready.active_provider, "local");

        let model_file = local_semantic::test_model_runtime_file(model_root.path(), "config.json");
        std::fs::write(model_file, b"tampered-content-1234567")?;

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.active_provider, "feature-hash");
        assert!(status.degraded);
        assert!(status
            .degradation_reason
            .as_deref()
            .unwrap_or_default()
            .contains("using feature-hash"));
        assert!(status.unavailable_reason.is_none());
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_runtime_constructor_failure_degrades_to_feature_hash() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        let _failure = local_semantic::fail_test_model_runtime_readiness(
            model_root.path(),
            "synthetic ONNX constructor failure",
        )?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.configured_provider, "auto");
        assert_eq!(status.active_provider, "feature-hash");
        assert!(status.degraded);
        assert!(status
            .degradation_reason
            .as_deref()
            .unwrap_or_default()
            .contains("initialize verified local embedding runtime"));
        assert!(status
            .degradation_reason
            .as_deref()
            .unwrap_or_default()
            .contains("synthetic ONNX constructor failure"));
        Ok(())
    })
}

#[test]
#[cfg(not(feature = "local-onnx"))]
fn auto_provider_does_not_select_manifest_without_local_runtime() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe { std::env::set_var(ENV_MODEL_DIR, model_root.path()) };

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.active_provider, "feature-hash");
        assert!(status.degraded);
        assert!(status
            .degradation_reason
            .as_deref()
            .unwrap_or_default()
            .contains("local-onnx"));
        Ok(())
    })
}

#[test]
fn auto_provider_prefers_api_key_over_installed_local_model() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe {
            std::env::set_var(ENV_MODEL_DIR, model_root.path());
            std::env::set_var(ENV_API_KEY, "test-key");
        }

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.configured_provider, "auto");
        assert_eq!(status.active_provider, "api");
        assert_eq!(
            status.active_model_id.as_deref(),
            Some(OPENAI_DEFAULT_MODEL)
        );
        Ok(())
    })
}

#[test]
fn auto_provider_prefers_configured_custom_api_key_env() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe {
            std::env::set_var(ENV_MODEL_DIR, model_root.path());
            std::env::set_var(ENV_API_KEY_ENV, TEST_API_KEY_ENV);
            std::env::set_var(TEST_API_KEY_ENV, "test-key");
        }

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.configured_provider, "auto");
        assert_eq!(status.active_provider, "api");
        assert_eq!(
            status.active_model_id.as_deref(),
            Some(OPENAI_DEFAULT_MODEL)
        );
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_uses_installed_e5_with_unrelated_hf_home() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe {
            std::env::set_var(ENV_MODEL_DIR, model_root.path());
            std::env::set_var("HF_HOME", model_root.path().join("other-hf-cache"));
        }

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.active_provider, "local");
        assert!(!status.degraded);
        assert!(status.degradation_reason.is_none());
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn auto_provider_uses_installed_e5_with_empty_hf_home() -> Result<()> {
    with_clean_env(|| {
        let model_root = TestModelRoot::new();
        install_test_local_embedding_model(model_root.path())?;
        unsafe {
            std::env::set_var(ENV_MODEL_DIR, model_root.path());
            std::env::set_var("HF_HOME", "");
        }

        let status = embedding_provider_status_without_probe()?;

        assert_eq!(status.active_provider, "local");
        assert!(!status.degraded);
        assert!(status.degradation_reason.is_none());
        Ok(())
    })
}

#[test]
fn explicit_openai_requires_api_key() {
    with_clean_env(|| {
        unsafe { std::env::set_var(ENV_PROVIDER, "openai") };

        let err = embed_query("hello").unwrap_err();

        assert!(err.to_string().contains("requires"));
    });
}

#[test]
fn config_file_selects_openai_without_secret_in_file() -> Result<()> {
    with_clean_env(|| {
        let path = std::env::temp_dir().join(format!(
            "remem-embedding-config-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(
            &path,
            r#"[embeddings]
provider = "openai"
model = "text-embedding-3-large"
base_url = "https://example.invalid/v1"
dimensions = 256
api_key_env = "REMEM_TEST_EMBEDDING_KEY"
"#,
        )?;
        unsafe {
            std::env::set_var("REMEM_CONFIG", &path);
            std::env::set_var(TEST_API_KEY_ENV, "test-key");
        }

        let config = resolve_embedding_config()?;
        let active = active_provider(&config)?;

        assert_eq!(config.provider, EmbeddingProvider::OpenAi);
        assert_eq!(config.fallback, None);
        assert_eq!(config.model, "text-embedding-3-large");
        assert_eq!(config.dimensions, Some(256));
        assert!(matches!(active, ActiveEmbeddingProvider::OpenAi { .. }));
        std::fs::remove_file(path).ok();
        Ok(())
    })
}

#[test]
fn local_and_feature_hash_are_distinct_configured_providers() -> Result<()> {
    with_clean_env(|| {
        let model_dir = std::env::temp_dir().join(format!(
            "remem-empty-local-models-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        unsafe {
            std::env::set_var(ENV_PROVIDER, "local");
            std::env::set_var(ENV_MODEL_DIR, &model_dir);
        }
        let local = resolve_embedding_config()?;
        let local_status = embedding_provider_status()?;
        assert_eq!(local.provider, EmbeddingProvider::Local);
        assert_eq!(local_status.configured_provider, "local");
        assert_eq!(local_status.active_provider, "local");
        assert_eq!(local_status.active_model_id, None);
        assert_eq!(local_status.active_dimensions, None);
        assert!(local_status
            .unavailable_reason
            .as_deref()
            .unwrap_or_default()
            .contains(if cfg!(feature = "local-onnx") {
                "local embedding model multilingual-e5-small is not ready"
            } else {
                "local-onnx"
            }));

        unsafe { std::env::set_var(ENV_PROVIDER, "feature-hash") };
        let feature_hash = resolve_embedding_config()?;
        let feature_hash_status = embedding_provider_status()?;
        assert_eq!(feature_hash.provider, EmbeddingProvider::FeatureHash);
        assert_eq!(feature_hash_status.configured_provider, "feature-hash");
        assert_eq!(feature_hash_status.active_provider, "feature-hash");
        assert_eq!(
            feature_hash_status.active_model_id.as_deref(),
            Some(LOCAL_EMBEDDING_MODEL)
        );
        Ok(())
    })
}

#[test]
fn off_provider_reports_disabled_and_refuses_embedding() {
    with_clean_env(|| {
        unsafe { std::env::set_var(ENV_PROVIDER, "off") };

        let status = embedding_provider_status().expect("status should resolve");
        let err = embed_query("hello").unwrap_err();

        assert_eq!(status.configured_provider, "off");
        assert_eq!(status.active_provider, "off");
        assert!(status.disabled);
        assert!(err.to_string().contains("provider is off"));
    });
}

#[test]
fn api_provider_without_key_uses_configured_fallback_visibly() -> Result<()> {
    with_clean_env(|| {
        unsafe {
            std::env::set_var(ENV_PROVIDER, "api");
            std::env::set_var(ENV_FALLBACK, "feature-hash");
        }

        let config = resolve_embedding_config()?;
        let active = active_provider(&config)?;
        let status = embedding_provider_status()?;

        assert_eq!(config.provider, EmbeddingProvider::OpenAi);
        assert_eq!(config.fallback, Some(EmbeddingProvider::FeatureHash));
        assert!(matches!(active, ActiveEmbeddingProvider::FeatureHash));
        assert!(status.degraded);
        assert!(!status.disabled);
        assert_eq!(status.active_provider, "feature-hash");
        assert!(status
            .degradation_reason
            .as_deref()
            .unwrap_or("")
            .contains("using fallback feature-hash"));
        Ok(())
    })
}

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
fn api_provider_call_failure_uses_configured_feature_hash_fallback() -> Result<()> {
    with_clean_env(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let handle = std::thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer)?;
            let body = "provider unavailable";
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });
        unsafe {
            std::env::set_var(ENV_PROVIDER, "api");
            std::env::set_var(ENV_FALLBACK, "feature-hash");
            std::env::set_var(ENV_API_KEY, "test-key");
            std::env::set_var(ENV_BASE_URL, format!("http://{addr}/v1"));
        }

        let embedding = embed_query("remote endpoint fallback")?;
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("embedding test server thread panicked"))??;

        assert_eq!(embedding.model(), LOCAL_EMBEDDING_MODEL);
        assert_eq!(embedding.dimensions(), LOCAL_EMBEDDING_DIMENSIONS);
        Ok(())
    })
}

#[test]
fn config_file_reads_fallback_and_model_dir() -> Result<()> {
    with_clean_env(|| {
        let path = std::env::temp_dir().join(format!(
            "remem-embedding-config-contract-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(
            &path,
            r#"[embeddings]
provider = "api"
fallback = "feature-hash"
model_dir = "/tmp/remem-models"
"#,
        )?;
        unsafe {
            std::env::set_var("REMEM_CONFIG", &path);
        }

        let config = resolve_embedding_config()?;

        assert_eq!(config.provider, EmbeddingProvider::OpenAi);
        assert_eq!(config.fallback, Some(EmbeddingProvider::FeatureHash));
        assert_eq!(config.model_dir.as_deref(), Some("/tmp/remem-models"));
        std::fs::remove_file(path).ok();
        Ok(())
    })
}

#[test]
fn local_inventory_ignores_api_model_when_provider_is_not_local() -> Result<()> {
    with_clean_env(|| {
        let model_dir = std::env::temp_dir().join(format!(
            "remem-api-model-inventory-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        unsafe {
            std::env::set_var(ENV_PROVIDER, "api");
            std::env::set_var(ENV_MODEL, "text-embedding-3-large");
            std::env::set_var(ENV_MODEL_DIR, &model_dir);
        }

        let inventory = local_embedding_inventory()?;

        assert_eq!(inventory.configured_preset, "multilingual-e5-small");
        assert_eq!(inventory.models.len(), 2);
        assert!(inventory
            .models
            .iter()
            .any(|model| model.preset == "multilingual-e5-small"));
        Ok(())
    })
}

#[test]
fn parses_openai_embedding_response() -> Result<()> {
    let embedding = parse_openai_embedding_response(
        r#"{"data":[{"embedding":[0.1,0.2,0.3]}],"model":"text-embedding-3-small"}"#,
        "fallback",
    )?;

    assert_eq!(embedding.model(), "text-embedding-3-small");
    assert_eq!(embedding.values(), &[0.1, 0.2, 0.3]);
    Ok(())
}

#[test]
fn backfill_target_uses_provider_returned_profile() -> Result<()> {
    with_clean_env(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let handle = std::thread::spawn(move || -> Result<String> {
            let (mut stream, _) = listener.accept()?;
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = r#"{"data":[{"embedding":[0.1,0.2,0.3,0.4]}],"model":"normalized-model"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(request)
        });
        unsafe {
            std::env::set_var(ENV_PROVIDER, "openai");
            std::env::set_var(ENV_API_KEY, "test-key");
            std::env::set_var(ENV_MODEL, "requested-model");
            std::env::set_var(ENV_DIMENSIONS, "256");
            std::env::set_var(ENV_BASE_URL, format!("http://{addr}/v1"));
        }

        let target = configured_backfill_target()?;
        let request = handle
            .join()
            .map_err(|_| anyhow::anyhow!("embedding test server thread panicked"))??;

        assert_eq!(target.model, "normalized-model");
        assert_eq!(target.dimensions, 4);
        assert!(request.contains("\"model\":\"requested-model\""));
        assert!(request.contains("\"dimensions\":256"));
        Ok(())
    })
}

#[test]
fn api_provider_status_uses_provider_returned_profile() -> Result<()> {
    with_clean_env(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let handle = std::thread::spawn(move || -> Result<String> {
            let (mut stream, _) = listener.accept()?;
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = r#"{"data":[{"embedding":[0.1,0.2,0.3,0.4]}],"model":"normalized-model"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(request)
        });
        unsafe {
            std::env::set_var(ENV_PROVIDER, "openai");
            std::env::set_var(ENV_API_KEY, "test-key");
            std::env::set_var(ENV_MODEL, "requested-model");
            std::env::set_var(ENV_DIMENSIONS, "256");
            std::env::set_var(ENV_BASE_URL, format!("http://{addr}/v1"));
        }

        let status = embedding_provider_status()?;
        let request = handle
            .join()
            .map_err(|_| anyhow::anyhow!("embedding test server thread panicked"))??;

        assert_eq!(status.active_model_id.as_deref(), Some("normalized-model"));
        assert_eq!(status.active_dimensions, Some(4));
        assert!(request.contains("\"model\":\"requested-model\""));
        assert!(request.contains("\"dimensions\":256"));
        Ok(())
    })
}

#[test]
fn truncates_provider_error_body_on_char_boundary() {
    let body = format!("{}猫", "x".repeat(499));

    let truncated = truncate_error_body(&body);

    assert!(truncated.ends_with("..."));
}

#[test]
fn openai_provider_calls_configured_embeddings_endpoint() -> Result<()> {
    with_clean_env(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let handle = std::thread::spawn(move || -> Result<String> {
            let (mut stream, _) = listener.accept()?;
            let mut buffer = [0u8; 8192];
            let read = stream.read(&mut buffer)?;
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = r#"{"data":[{"embedding":[0.4,0.5,0.6]}],"model":"test-embedding"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(request)
        });
        unsafe {
            std::env::set_var(ENV_PROVIDER, "openai");
            std::env::set_var(ENV_API_KEY, "test-key");
            std::env::set_var(ENV_MODEL, "test-embedding");
            std::env::set_var(ENV_BASE_URL, format!("http://{addr}/v1"));
        }

        let embedding = embed_query("remote semantic text")?;
        let request = handle
            .join()
            .map_err(|_| anyhow::anyhow!("embedding test server thread panicked"))??;

        assert_eq!(embedding.model(), "test-embedding");
        assert_eq!(embedding.values(), &[0.4, 0.5, 0.6]);
        assert!(request.starts_with("POST /v1/embeddings "));
        assert!(request.contains("authorization: Bearer test-key"));
        assert!(request.contains("\"model\":\"test-embedding\""));
        Ok(())
    })
}
