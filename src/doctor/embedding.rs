use rusqlite::Connection;

use super::types::{Check, Status};

const MIN_ACTIVE_MODEL_COVERAGE_PERCENT: f64 = 90.0;

pub(super) fn check_embedding_provider(conn: Option<&Connection>) -> Vec<Check> {
    let status = match crate::retrieval::embedding::embedding_provider_status() {
        Ok(status) => status,
        Err(error) => {
            return vec![Check::new(
                "Embedding provider",
                Status::Fail,
                format!("embedding config invalid: {error}"),
            )];
        }
    };

    let mut checks = vec![provider_check(&status)];
    if status.disabled {
        checks.push(Check::new(
            "Embedding coverage",
            Status::Ok,
            "provider=off; vector coverage is intentionally ignored",
        ));
        return checks;
    }
    if status.unavailable_reason.is_some() {
        return checks;
    }

    let Some(conn) = conn else {
        checks.push(Check::new(
            "Embedding coverage",
            Status::Warn,
            "database unavailable; active-model vector coverage not inspected",
        ));
        return checks;
    };

    match crate::retrieval::vector::active_embedding_coverage_for_status(conn, &status) {
        Ok(coverage) => {
            checks.push(coverage_check(&status, &coverage));
            checks.push(mixed_model_check(coverage.mixed_profile_count));
        }
        Err(error) => checks.push(Check::new(
            "Embedding coverage",
            Status::Warn,
            format!("active-model vector coverage unavailable: {error}"),
        )),
    }
    checks
}

fn provider_check(status: &crate::retrieval::embedding::EmbeddingProviderStatus) -> Check {
    if let Some(reason) = &status.unavailable_reason {
        return Check::new(
            "Embedding provider",
            Status::Fail,
            format!(
                "configured provider={} unavailable: {}",
                status.configured_provider, reason
            ),
        );
    }
    if status.degraded {
        return Check::new(
            "Embedding provider",
            Status::Warn,
            status
                .degradation_reason
                .clone()
                .unwrap_or_else(|| "embedding provider fallback active".to_string()),
        );
    }
    if status.disabled {
        return Check::new(
            "Embedding provider",
            Status::Ok,
            "provider=off; vector channel disabled explicitly",
        );
    }
    Check::new(
        "Embedding provider",
        Status::Ok,
        format!(
            "configured={} active={} model={}",
            status.configured_provider,
            status.active_provider,
            status.active_model_id.as_deref().unwrap_or("none")
        ),
    )
}

fn coverage_check(
    status: &crate::retrieval::embedding::EmbeddingProviderStatus,
    coverage: &crate::retrieval::vector::ActiveEmbeddingCoverage,
) -> Check {
    if coverage.total == 0 || coverage.percent >= MIN_ACTIVE_MODEL_COVERAGE_PERCENT {
        return Check::new(
            "Embedding coverage",
            Status::Ok,
            format!(
                "{}/{} memories have vectors for active model {} ({:.1}%)",
                coverage.embedded,
                coverage.total,
                status.active_model_id.as_deref().unwrap_or("none"),
                coverage.percent
            ),
        );
    }
    Check::new(
        "Embedding coverage",
        Status::Warn,
        format!(
            "{}/{} memories have vectors for active model {} ({:.1}%); run `remem reindex-embeddings --limit 1000`",
            coverage.embedded,
            coverage.total,
            status.active_model_id.as_deref().unwrap_or("none"),
            coverage.percent
        ),
    )
}

fn mixed_model_check(mixed_profile_count: i64) -> Check {
    if mixed_profile_count <= 1 {
        return Check::new(
            "Embedding model mix",
            Status::Ok,
            format!("{mixed_profile_count} embedding model/dimension profile(s) present"),
        );
    }
    Check::new(
        "Embedding model mix",
        Status::Warn,
        format!(
            "{mixed_profile_count} embedding model/dimension profiles present; backfill the active model before relying on vector recall"
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use rusqlite::{params, Connection};

    use super::*;

    const ENV_KEYS: &[&str] = &[
        "REMEM_CONFIG",
        "REMEM_EMBEDDINGS_PROVIDER",
        "REMEM_EMBEDDING_PROVIDER",
        "REMEM_EMBEDDINGS_MODEL",
        "REMEM_EMBEDDING_MODEL",
        "REMEM_EMBEDDINGS_DIMENSIONS",
        "REMEM_EMBEDDING_DIMENSIONS",
        "REMEM_EMBEDDINGS_BASE_URL",
        "REMEM_EMBEDDING_BASE_URL",
        "REMEM_EMBEDDINGS_FALLBACK",
        "REMEM_EMBEDDINGS_API_KEY",
        "REMEM_EMBEDDING_API_KEY",
        "REMEM_EMBEDDINGS_API_KEY_ENV",
        "REMEM_EMBEDDINGS_TIMEOUT_SECS",
        "REMEM_EMBEDDINGS_MODEL_DIR",
        "OPENAI_API_KEY",
    ];

    fn with_clean_embedding_env<T>(f: impl FnOnce() -> T) -> T {
        let _guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("env lock should acquire");
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

    fn setup_conn() -> anyhow::Result<Connection> {
        let conn = Connection::open_in_memory()?;
        crate::migrate::run_migrations(&conn)?;
        Ok(conn)
    }

    struct SuccessfulEmbeddingServer {
        base_url: String,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
    }

    impl SuccessfulEmbeddingServer {
        fn start(body: &'static str) -> anyhow::Result<Self> {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            let addr = listener.local_addr()?;
            let stop = Arc::new(AtomicBool::new(false));
            let stop_for_thread = Arc::clone(&stop);
            let handle = std::thread::spawn(move || -> anyhow::Result<()> {
                while !stop_for_thread.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let mut buffer = [0_u8; 8192];
                            let _ = stream.read(&mut buffer)?;
                            let response = format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            );
                            stream.write_all(response.as_bytes())?;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Ok(())
            });
            Ok(Self {
                base_url: format!("http://{addr}/v1"),
                stop,
                handle: Some(handle),
            })
        }
    }

    impl Drop for SuccessfulEmbeddingServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                handle
                    .join()
                    .expect("embedding server thread should not panic")
                    .expect("embedding server should stop cleanly");
            }
        }
    }

    #[test]
    fn provider_check_warns_when_api_falls_back_to_feature_hash() {
        with_clean_embedding_env(|| {
            unsafe {
                std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
                std::env::set_var("REMEM_EMBEDDINGS_FALLBACK", "feature-hash");
            }

            let checks = check_embedding_provider(None);

            assert!(matches!(checks[0].status, Status::Warn));
            assert!(checks[0].detail.contains("using fallback feature-hash"));
        });
    }

    #[test]
    fn provider_check_fails_when_configured_api_is_unavailable_without_fallback() {
        with_clean_embedding_env(|| {
            unsafe {
                std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
            }

            let checks = check_embedding_provider(None);

            assert!(matches!(checks[0].status, Status::Fail));
            assert!(checks[0].detail.contains("unavailable"));
        });
    }

    #[test]
    fn off_provider_skips_coverage_warning() {
        with_clean_embedding_env(|| {
            unsafe {
                std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "off");
            }

            let checks = check_embedding_provider(None);

            assert!(matches!(checks[0].status, Status::Ok));
            assert!(matches!(checks[1].status, Status::Ok));
            assert!(checks[1].detail.contains("intentionally ignored"));
        });
    }

    #[test]
    fn fallback_to_off_reports_degraded_provider_before_coverage_skip() {
        with_clean_embedding_env(|| {
            unsafe {
                std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
                std::env::set_var("REMEM_EMBEDDINGS_FALLBACK", "off");
            }

            let checks = check_embedding_provider(None);

            assert!(matches!(checks[0].status, Status::Warn));
            assert!(checks[0].detail.contains("using fallback off"));
            assert!(matches!(checks[1].status, Status::Ok));
            assert!(checks[1].detail.contains("intentionally ignored"));
        });
    }

    #[test]
    fn coverage_check_warns_when_active_model_vectors_are_low() -> anyhow::Result<()> {
        with_clean_embedding_env(|| -> anyhow::Result<()> {
            unsafe {
                std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "feature-hash");
            }
            let conn = setup_conn()?;
            for id in 1..=10 {
                conn.execute(
                    "INSERT INTO memories
                     (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
                     VALUES (?1, '/repo', 'Memory', 'Content', 'decision', 1, ?1, 'active')",
                    params![id],
                )?;
            }
            crate::retrieval::vector::upsert_memory_embedding(
                &conn, 1, "Memory", "Content", "decision", None,
            )?;

            let checks = check_embedding_provider(Some(&conn));

            let coverage = checks
                .iter()
                .find(|check| check.name == "Embedding coverage")
                .expect("coverage check should exist");
            assert!(matches!(coverage.status, Status::Warn));
            assert!(coverage.detail.contains("1/10"));
            Ok(())
        })
    }

    #[test]
    fn api_coverage_uses_provider_returned_profile() -> anyhow::Result<()> {
        with_clean_embedding_env(|| -> anyhow::Result<()> {
            let server = SuccessfulEmbeddingServer::start(
                r#"{"data":[{"embedding":[0.1,0.2,0.3,0.4]}],"model":"normalized-model"}"#,
            )?;
            unsafe {
                std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "api");
                std::env::set_var("REMEM_EMBEDDINGS_API_KEY", "test-key");
                std::env::set_var("REMEM_EMBEDDINGS_MODEL", "requested-model");
                std::env::set_var("REMEM_EMBEDDINGS_DIMENSIONS", "256");
                std::env::set_var("REMEM_EMBEDDINGS_BASE_URL", &server.base_url);
            }
            let conn = setup_conn()?;
            conn.execute(
                "INSERT INTO memories
                 (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
                 VALUES (1, '/repo', 'Memory', 'Content', 'decision', 1, 1, 'active')",
                [],
            )?;
            conn.execute(
                "INSERT INTO memory_embeddings
                 (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
                 VALUES (1, ?1, 4, 'normalized-model', 'hash', 1)",
                params![vec![0_u8; 16]],
            )?;

            let checks = check_embedding_provider(Some(&conn));
            let coverage = checks
                .iter()
                .find(|check| check.name == "Embedding coverage")
                .ok_or_else(|| anyhow::anyhow!("coverage check should exist"))?;

            assert!(matches!(coverage.status, Status::Ok));
            assert!(coverage.detail.contains("1/1"));
            assert!(coverage.detail.contains("normalized-model"));
            Ok(())
        })
    }

    #[test]
    fn mixed_model_check_warns_when_multiple_profiles_exist() -> anyhow::Result<()> {
        with_clean_embedding_env(|| -> anyhow::Result<()> {
            unsafe {
                std::env::set_var("REMEM_EMBEDDINGS_PROVIDER", "feature-hash");
            }
            let conn = setup_conn()?;
            for (id, model) in [(1_i64, "remem-local-feature-hash-v1"), (2, "old-model")] {
                conn.execute(
                    "INSERT INTO memories
                     (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
                     VALUES (?1, '/repo', 'Memory', 'Content', 'decision', 1, ?1, 'active')",
                    params![id],
                )?;
                conn.execute(
                    "INSERT INTO memory_embeddings
                     (memory_id, embedding, dimensions, model, content_hash, updated_at_epoch)
                     VALUES (?1, ?2, ?3, ?4, 'hash', 1)",
                    params![
                        id,
                        vec![0_u8; crate::retrieval::embedding::LOCAL_EMBEDDING_DIMENSIONS * 4],
                        crate::retrieval::embedding::LOCAL_EMBEDDING_DIMENSIONS as i64,
                        model
                    ],
                )?;
            }

            let checks = check_embedding_provider(Some(&conn));

            let model_mix = checks
                .iter()
                .find(|check| check.name == "Embedding model mix")
                .expect("model mix check should exist");
            assert!(matches!(model_mix.status, Status::Warn));
            assert!(model_mix.detail.contains("2 embedding model"));
            Ok(())
        })
    }
}
