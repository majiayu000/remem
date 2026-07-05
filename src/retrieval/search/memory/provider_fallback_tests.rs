use std::io::{Read, Write};

use anyhow::Result;
use rusqlite::Connection;

use super::search_with_branch_explain;

const ENV_KEYS: &[&str] = &[
    "REMEM_CONFIG",
    "REMEM_EMBEDDINGS_PROVIDER",
    "REMEM_EMBEDDING_PROVIDER",
    "REMEM_EMBEDDINGS_FALLBACK",
    "REMEM_EMBEDDINGS_API_KEY",
    "REMEM_EMBEDDING_API_KEY",
    "REMEM_EMBEDDINGS_BASE_URL",
    "REMEM_EMBEDDING_BASE_URL",
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
