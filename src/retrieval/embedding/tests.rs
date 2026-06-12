use std::io::{Read, Write};
use std::sync::Mutex;

use super::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());
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
    DEFAULT_API_KEY_ENV,
    TEST_API_KEY_ENV,
];

fn with_clean_env<T>(f: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().expect("env lock should acquire");
    let saved = ENV_KEYS
        .iter()
        .map(|key| (*key, std::env::var(key).ok()))
        .collect::<Vec<_>>();
    for key in ENV_KEYS {
        unsafe { std::env::remove_var(key) };
    }
    let result = f();
    for (key, value) in saved {
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }
    result
}

#[test]
fn auto_provider_uses_local_without_remem_specific_key() -> Result<()> {
    with_clean_env(|| {
        let embedding = embed_query("protect persisted data")?;

        assert_eq!(embedding.model(), LOCAL_EMBEDDING_MODEL);
        assert_eq!(embedding.dimensions(), LOCAL_EMBEDDING_DIMENSIONS);
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
        assert_eq!(config.model, "text-embedding-3-large");
        assert_eq!(config.dimensions, Some(256));
        assert!(matches!(active, ActiveEmbeddingProvider::OpenAi { .. }));
        std::fs::remove_file(path).ok();
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
