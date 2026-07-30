use std::io::{Read, Write};
#[cfg(feature = "local-onnx")]
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::{search_with_branch_explain, SearchExplain, SearchExplainChannel};

const ENV_KEYS: &[&str] = &[
    "REMEM_CONFIG",
    "REMEM_EMBEDDINGS_PROVIDER",
    "REMEM_EMBEDDING_PROVIDER",
    "REMEM_EMBEDDINGS_FALLBACK",
    "REMEM_EMBEDDINGS_API_KEY",
    "REMEM_EMBEDDING_API_KEY",
    "REMEM_EMBEDDINGS_BASE_URL",
    "REMEM_EMBEDDING_BASE_URL",
    "REMEM_EMBEDDINGS_MODEL",
    "REMEM_EMBEDDING_MODEL",
    "REMEM_EMBEDDINGS_DIMENSIONS",
    "REMEM_EMBEDDING_DIMENSIONS",
    "REMEM_EMBEDDINGS_MODEL_DIR",
    "HF_HOME",
    "HF_ENDPOINT",
    "OPENAI_API_KEY",
];

struct ScopedSearchEmbeddingEnv {
    _guard: crate::runtime_config::TestEnvGuard,
    saved: Vec<(&'static str, Option<String>)>,
}

impl ScopedSearchEmbeddingEnv {
    fn clean() -> Self {
        let guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
        let saved = ENV_KEYS
            .iter()
            .map(|key| (*key, std::env::var(key).ok()))
            .collect::<Vec<_>>();
        for key in ENV_KEYS {
            unsafe { std::env::remove_var(key) };
        }
        let isolated_config = std::env::temp_dir().join(format!(
            "remem-search-embedding-missing-config-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        unsafe { std::env::set_var("REMEM_CONFIG", isolated_config) };
        Self {
            _guard: guard,
            saved,
        }
    }
}

impl Drop for ScopedSearchEmbeddingEnv {
    fn drop(&mut self) {
        for (key, value) in self.saved.drain(..) {
            match value {
                Some(value) => unsafe { std::env::set_var(key, value) },
                None => unsafe { std::env::remove_var(key) },
            }
        }
    }
}

fn with_clean_search_embedding_env<T>(f: impl FnOnce() -> T) -> T {
    let _env = ScopedSearchEmbeddingEnv::clean();
    f()
}

fn setup_search_conn() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    crate::memory::tests_helper::setup_memory_schema(&conn);
    Ok(conn)
}

fn insert_search_memory(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, scope)
         VALUES (1, 'session-1', '/repo', 'Semantic fallback',
                 'FTS result survives provider failure.', 'decision',
                 1, 1, 'active', 'project')",
        [],
    )?;
    Ok(())
}

fn explain_for_query(conn: &Connection, query: &str) -> Result<SearchExplain> {
    let (_memories, explain) =
        search_with_branch_explain(conn, Some(query), Some("/repo"), None, 5, 0, false, None)?;
    explain.context("query explain should be present")
}

fn vector_channel(explain: &SearchExplain) -> Result<&SearchExplainChannel> {
    explain
        .channels
        .iter()
        .find(|channel| channel.name == "vector")
        .context("vector channel should be reported")
}

#[cfg(feature = "local-onnx")]
struct TestModelRoot(PathBuf);

#[cfg(feature = "local-onnx")]
impl TestModelRoot {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "remem-search-embedding-model-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        )))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(feature = "local-onnx")]
impl Drop for TestModelRoot {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "failed to remove search embedding test model root {}: {error}",
                    self.0.display()
                );
            }
        }
    }
}

#[test]
fn search_explain_reports_actual_api_embedding_profile() -> Result<()> {
    with_clean_search_embedding_env(|| -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let handle = std::thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut buffer = [0_u8; 8192];
            let _ = stream.read(&mut buffer)?;
            let body = r#"{"data":[{"embedding":[0.1,0.2,0.3]}],"model":"actual-api-profile"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes())?;
            Ok(())
        });
        unsafe {
            std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
            std::env::set_var("REMEM_EMBEDDINGS_MODEL", "configured-api-profile");
            std::env::set_var("REMEM_EMBEDDINGS_API_KEY", "test-key");
            std::env::set_var("REMEM_EMBEDDINGS_BASE_URL", format!("http://{addr}/v1"));
        }
        let conn = setup_search_conn()?;
        insert_search_memory(&conn)?;

        let explain = explain_for_query(&conn, "Semantic fallback")?;
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("embedding test server thread panicked"))??;
        let vector = vector_channel(&explain)?;
        let embedding = vector
            .embedding
            .as_ref()
            .context("vector channel should expose embedding execution")?;

        assert_eq!(embedding.configured_provider, "api");
        assert_eq!(embedding.active_provider, "api");
        assert_eq!(embedding.model, "actual-api-profile");
        assert_eq!(embedding.dimensions, 3);
        assert!(!embedding.degraded);
        assert!(embedding.degradation_reason.is_none());

        let serialized = serde_json::to_value(&explain)?;
        let fts = serialized["channels"]
            .as_array()
            .and_then(|channels| {
                channels
                    .iter()
                    .find(|channel| channel["name"].as_str() == Some("fts"))
            })
            .context("fts channel should be serialized")?;
        assert!(
            fts.get("embedding").is_none(),
            "non-vector channels must remain compatible: {fts}"
        );
        Ok(())
    })
}

#[test]
#[cfg(all(unix, feature = "local-onnx"))]
fn search_explain_reports_initial_auto_feature_hash_degradation() -> Result<()> {
    use std::os::unix::fs::symlink;

    with_clean_search_embedding_env(|| -> Result<()> {
        let model_root = TestModelRoot::new();
        std::fs::create_dir_all(model_root.path())?;
        symlink(
            model_root.path().join("missing-install-target"),
            model_root
                .path()
                .join(crate::retrieval::embedding::TEST_LOCAL_SEMANTIC_MODEL),
        )?;
        unsafe {
            std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "auto");
            std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", model_root.path());
        }
        let conn = setup_search_conn()?;

        let explain = explain_for_query(&conn, "initial degraded vector query")?;
        let embedding = vector_channel(&explain)?
            .embedding
            .as_ref()
            .context("degraded vector channel should expose embedding execution")?;

        assert_eq!(embedding.configured_provider, "auto");
        assert_eq!(embedding.active_provider, "feature-hash");
        assert_eq!(
            embedding.model,
            crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL
        );
        assert_eq!(
            embedding.dimensions,
            crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_DIMENSIONS
        );
        assert!(embedding.degraded);
        assert!(
            embedding
                .degradation_reason
                .as_deref()
                .unwrap_or_default()
                .contains("symlink"),
            "{embedding:#?}"
        );
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn search_explain_reports_runtime_local_unavailable_feature_hash_fallback() -> Result<()> {
    with_clean_search_embedding_env(|| -> Result<()> {
        let model_root = TestModelRoot::new();
        crate::retrieval::embedding::install_test_local_embedding_model(model_root.path())?;
        let _failure = crate::retrieval::embedding::fail_next_test_model_embed_unavailable(
            model_root.path(),
            "synthetic local runtime loss token=runtime-secret-value",
        )?;
        unsafe {
            std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "auto");
            std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", model_root.path());
        }
        let conn = setup_search_conn()?;

        let explain = explain_for_query(&conn, "runtime embedding race")?;
        let embedding = vector_channel(&explain)?
            .embedding
            .as_ref()
            .context("runtime fallback should expose embedding execution")?;
        let reason = embedding.degradation_reason.as_deref().unwrap_or_default();

        assert_eq!(embedding.configured_provider, "auto");
        assert_eq!(embedding.active_provider, "feature-hash");
        assert_eq!(
            embedding.model,
            crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_MODEL
        );
        assert_eq!(
            embedding.dimensions,
            crate::retrieval::embedding::FEATURE_HASH_EMBEDDING_DIMENSIONS
        );
        assert!(embedding.degraded);
        assert!(
            reason.contains("automatic local embedding provider became unavailable"),
            "{reason}"
        );
        assert!(reason.contains("[REDACTED]"), "{reason}");
        assert!(!reason.contains("runtime-secret-value"), "{reason}");
        Ok(())
    })
}

#[test]
#[cfg(feature = "local-onnx")]
fn search_explain_keeps_generic_local_inference_errors_loud() -> Result<()> {
    with_clean_search_embedding_env(|| -> Result<()> {
        let model_root = TestModelRoot::new();
        crate::retrieval::embedding::install_test_local_embedding_model(model_root.path())?;
        let _failure = crate::retrieval::embedding::fail_next_test_model_embed_generic(
            model_root.path(),
            "synthetic generic local inference failure",
        )?;
        unsafe {
            std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "auto");
            std::env::set_var("REMEM_EMBEDDINGS_MODEL_DIR", model_root.path());
        }
        let conn = setup_search_conn()?;

        let error = explain_for_query(&conn, "generic inference race")
            .expect_err("generic inference errors must not degrade to feature-hash");
        let message = format!("{error:#}");

        assert!(
            message.contains("synthetic generic local inference failure"),
            "{message}"
        );
        assert!(!message.contains("using feature-hash"), "{message}");
        Ok(())
    })
}

#[test]
fn search_returns_provider_error_when_api_failure_falls_back_to_off() -> Result<()> {
    with_clean_search_embedding_env(|| -> Result<()> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let handle = std::thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            let mut buffer = [0_u8; 8192];
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
            std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
            std::env::set_var("REMEM_EMBEDDINGS_FALLBACK", "off");
            std::env::set_var("REMEM_EMBEDDINGS_API_KEY", "test-key");
            std::env::set_var("REMEM_EMBEDDINGS_BASE_URL", format!("http://{addr}/v1"));
        }
        let conn = setup_search_conn()?;
        insert_search_memory(&conn)?;

        let error = search_with_branch_explain(
            &conn,
            Some("Semantic fallback"),
            Some("/repo"),
            None,
            5,
            0,
            false,
            None,
        )
        .expect_err("fallback=off after an API failure must not skip vector search errors");
        handle
            .join()
            .map_err(|_| anyhow::anyhow!("embedding test server thread panicked"))??;
        let error = format!("{error:#}");
        assert!(error.contains("provider unavailable"));
        assert!(error.contains("fallback off disabled provider fallback"));
        Ok(())
    })
}

#[test]
fn search_returns_provider_error_when_fallback_off_provider_is_unavailable_before_call(
) -> Result<()> {
    with_clean_search_embedding_env(|| -> Result<()> {
        unsafe {
            std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
            std::env::set_var("REMEM_EMBEDDINGS_FALLBACK", "off");
        }
        let conn = setup_search_conn()?;
        insert_search_memory(&conn)?;

        let error = search_with_branch_explain(
            &conn,
            Some("Semantic fallback"),
            Some("/repo"),
            None,
            5,
            0,
            false,
            None,
        )
        .expect_err("fallback=off provider status failures must not disable vector search");
        let error = format!("{error:#}");
        assert!(error.contains("requires REMEM_EMBEDDINGS_API_KEY"));
        assert!(error.contains("fallback off disabled provider fallback"));
        Ok(())
    })
}
