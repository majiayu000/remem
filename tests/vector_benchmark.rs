use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection};

use remem::{db, migrate, retrieval::vector};

const CHILD_ROOT_ENV: &str = "REMEM_VECTOR_BENCHMARK_CHILD_ROOT";

struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Result<Self> {
        let root = std::env::temp_dir().join(format!(
            "remem-vector-benchmark-{}-{}",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        // Claim a new directory before acquiring cleanup ownership.
        std::fs::create_dir(&root)?;
        let mut sandbox = Self(root);
        sandbox.0 = std::fs::canonicalize(&sandbox.0)?;
        for name in ["home", "data", "tmp"] {
            std::fs::create_dir(sandbox.0.join(name))?;
        }
        Ok(sandbox)
    }

    fn command(&self) -> Result<Command> {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "vector_search_10k_candidate_gate",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env(CHILD_ROOT_ENV, &self.0)
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("XDG_CONFIG_HOME", self.0.join("home/config"))
            .env("XDG_CACHE_HOME", self.0.join("home/cache"))
            .env("XDG_DATA_HOME", self.0.join("home/data"))
            .env("APPDATA", self.0.join("home/appdata"))
            .env("LOCALAPPDATA", self.0.join("home/localappdata"))
            .env("REMEM_DATA_DIR", self.0.join("data"))
            .env("REMEM_CONFIG", self.0.join("config.toml"))
            .env("REMEM_ALLOW_PLAINTEXT_DB", "1")
            .env("REMEM_EMBEDDINGS_PROVIDER", "feature-hash")
            .env("TEMP", self.0.join("tmp"))
            .env("TMP", self.0.join("tmp"))
            .env("TMPDIR", self.0.join("tmp"))
            .current_dir(&self.0);
        // Retain only process-loader settings needed by the compiled test binary.
        for key in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        Ok(command)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn vector_search_10k_candidate_gate() -> Result<()> {
    if let Some(root) = std::env::var_os(CHILD_ROOT_ENV) {
        return run_candidate_gate(Path::new(&root));
    }

    let sandbox = Sandbox::new()?;
    let output = sandbox
        .command()?
        .output()
        .context("run vector benchmark child")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics: Vec<&str> = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|line| line.starts_with("[VectorBound]"))
        .collect();
    ensure!(
        output.status.success()
            && diagnostics.len() == 2
            && diagnostics
                .iter()
                .any(|line| line.starts_with("[VectorBound] phase=exact "))
            && diagnostics
                .iter()
                .any(|line| line.starts_with("[VectorBound] phase=knn ")),
        "vector benchmark child failed: {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        output.status
    );
    // Forward benchmark diagnostics, without duplicating the child's libtest totals.
    for line in diagnostics {
        eprintln!("{line}");
    }
    Ok(())
}

fn assert_oldest_unique_match(outcome: &vector::VectorSearchOutcome) {
    assert_eq!(outcome.disabled_reason, None);
    assert_eq!(outcome.hits.len(), 10);
    assert_eq!(outcome.hits[0].memory_id, 1);
    assert_eq!(outcome.hits[0].distance, 0.0);
    assert!(outcome.hits.iter().skip(1).all(|hit| hit.distance > 0.0));
}

fn run_candidate_gate(root: &Path) -> Result<()> {
    ensure!(
        std::fs::canonicalize(std::env::current_dir()?)? == root,
        "child must run in its sandbox"
    );
    let db_path = db::try_db_path()?;
    ensure!(
        db_path == root.join("data/remem.db"),
        "child database escaped its sandbox"
    );

    // A plain connection deliberately has no sqlite-vec mirror. The same source
    // rows must remain fully searchable before the real DB opener builds it.
    let conn = Connection::open(&db_path)?;
    migrate::run_migrations(&conn)?;
    let mut query = vec![0.0_f32; vector::EMBEDDING_DIMENSIONS];
    query[0] = 1.0;
    let mut decoy = vec![0.0_f32; vector::EMBEDDING_DIMENSIONS];
    decoy[1] = 1.0;

    conn.execute("BEGIN IMMEDIATE", [])?;
    for id in 1..=10_000_i64 {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Vector bench', 'Bounded vector scan candidate', 'decision', ?1, ?1, 'active')",
            params![id],
        )?;
        let embedding = if id == 1 { &query } else { &decoy };
        vector::upsert_embedding(&conn, id, embedding)?;
    }
    conn.execute("COMMIT", [])?;

    let search = |conn: &Connection| {
        vector::vector_search_filtered(
            conn,
            &query,
            vector::VectorSearchFilters {
                project: Some("/repo"),
                ..vector::VectorSearchFilters::default()
            },
            10,
        )
    };
    let start = Instant::now();
    let fallback = search(&conn)?;
    eprintln!(
        "[VectorBound] phase=exact corpus=10000 scanned={} returned={} elapsed_ms={}",
        fallback.candidates_scanned,
        fallback.hits.len(),
        start.elapsed().as_millis()
    );
    assert!(fallback
        .timings
        .iter()
        .any(|timing| timing.phase == "vector_exact_scan"));
    assert_eq!(fallback.candidates_scanned, 10_000);
    assert_oldest_unique_match(&fallback);
    drop(conn);

    // Each real open advances one bounded batch. Read state only to observe
    // progress; do not manufacture a mirror or mark an incomplete one ready.
    let mut previous_cursor = 0_i64;
    let mut ready = None;
    for opens in 1..=32 {
        let conn = db::open_db_no_migrate().with_context(|| format!("backfill open {opens}"))?;
        let (cursor, done): (i64, i64) = conn
            .query_row(
                "SELECT last_memory_id, done FROM memory_embedding_vec_state_v2
             WHERE model = ?1 AND dimensions = ?2",
                params![
                    vector::DEFAULT_EMBEDDING_MODEL,
                    vector::EMBEDDING_DIMENSIONS as i64
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .with_context(|| format!("read backfill state after open {opens}"))?;
        ensure!(
            (0..=10_000).contains(&cursor),
            "invalid backfill cursor {cursor}"
        );
        ensure!(done == 0 || done == 1, "invalid backfill done value {done}");
        if done == 1 {
            assert_eq!(
                cursor, 10_000,
                "backfill completed before the last source row"
            );
            ready = Some((conn, opens));
            break;
        }
        ensure!(cursor > previous_cursor,
            "backfill did not progress on open {opens}: previous={previous_cursor}, cursor={cursor}");
        previous_cursor = cursor;
    }
    let (conn, opens) = ready.with_context(|| {
        format!("backfill did not finish within 32 real opens; last cursor={previous_cursor}")
    })?;

    let start = Instant::now();
    let outcome = search(&conn)?;
    eprintln!(
        "[VectorBound] phase=knn corpus=10000 scanned={} returned={} opens={} elapsed_ms={}",
        outcome.candidates_scanned,
        outcome.hits.len(),
        opens,
        start.elapsed().as_millis()
    );
    assert!(outcome
        .timings
        .iter()
        .any(|timing| timing.phase == "vector_knn_index"));
    assert!(outcome
        .timings
        .iter()
        .all(|timing| timing.phase != "vector_exact_scan"));
    assert!(outcome.candidates_scanned <= vector::VECTOR_SEARCH_CANDIDATE_LIMIT);
    assert!(outcome.candidates_scanned < 10_000);
    assert!(outcome.hits.len() <= 10);
    assert_oldest_unique_match(&outcome);
    Ok(())
}

#[test]
fn vector_search_10k_exact_fallback_preserves_oldest_best_match() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    migrate::run_migrations(&conn)?;
    let mut query = vec![0.0_f32; vector::EMBEDDING_DIMENSIONS];
    query[0] = 1.0;
    let mut other = vec![0.0_f32; vector::EMBEDDING_DIMENSIONS];
    other[1] = 1.0;

    conn.execute("BEGIN IMMEDIATE", [])?;
    for id in 1..=10_000_i64 {
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', 'Vector bench', 'Exact vector scan candidate', 'decision', ?1, ?1, 'active')",
            params![id],
        )?;
        let embedding = if id == 1 { &query } else { &other };
        vector::upsert_embedding(&conn, id, embedding)?;
    }
    // Make ID 1 oldest in both canonical and embedding recency order.
    conn.execute(
        "UPDATE memory_embeddings SET updated_at_epoch = memory_id",
        [],
    )?;
    conn.execute("COMMIT", [])?;
    assert_eq!(vector::embedding_count(&conn)?, 10_000);

    // Missing readiness state forces exact fallback even if sqlite-vec is
    // registered on this connection. Only derived state in this fixture is removed.
    conn.execute_batch("DROP TABLE IF EXISTS memory_embedding_vec_state_v2")?;

    let start = std::time::Instant::now();
    let outcome = vector::vector_search_filtered(
        &conn,
        &query,
        vector::VectorSearchFilters {
            project: Some("/repo"),
            ..vector::VectorSearchFilters::default()
        },
        10,
    )?;
    let elapsed = start.elapsed();
    eprintln!(
        "[VectorExactFallback] corpus=10000 scanned={} returned={} elapsed_ms={}",
        outcome.candidates_scanned,
        outcome.hits.len(),
        elapsed.as_millis()
    );

    assert!(outcome.disabled_reason.is_none(), "{outcome:?}");
    assert!(outcome
        .timings
        .iter()
        .any(|timing| timing.phase == "vector_exact_scan"));
    assert_eq!(outcome.candidates_scanned, 10_000);
    assert_eq!(outcome.hits.len(), 10);
    assert_eq!(outcome.hits[0].memory_id, 1);
    assert_eq!(outcome.hits[0].distance, 0.0);
    assert!(outcome.hits[1..]
        .iter()
        .all(|hit| hit.memory_id != 1 && hit.distance == 1.0));
    assert_eq!(
        outcome
            .hits
            .iter()
            .map(|hit| hit.memory_id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        10
    );
    Ok(())
}
