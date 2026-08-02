use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Instant;

use remem::{
    migrate,
    perf::format_phase_timings,
    retrieval::{search, vector},
};

const SQLITE_TUNING_CHILD_ENV: &str = "REMEM_SQLITE_TUNING_TEST_CHILD";
const SQLITE_TUNING_RESULT_ENV: &str = "REMEM_SQLITE_TUNING_RESULT_PATH";
const SQLITE_TUNING_MARKER_ENV: &str = "REMEM_SQLITE_TUNING_MARKER";
const SQLITE_CACHE_KIB_ENV: &str = "REMEM_SQLITE_CACHE_KIB";
const SQLITE_SYNCHRONOUS_ENV: &str = "REMEM_SQLITE_SYNCHRONOUS";
const SQLITE_BENCH_ROWS: usize = 12_000;
const SQLITE_BENCH_PAYLOAD_BYTES: usize = 4_096;

#[test]
#[ignore = "encrypted SQLite release A/B; run explicitly with --release --ignored --nocapture"]
fn sqlite_tuning_encrypted_release_ab_reports_latency() -> Result<()> {
    let sandbox = BenchmarkSandbox::new("sqlite-tuning-ab")?;
    let passphrase = initialize_encrypted_sandbox(&sandbox)?;
    let rows = SQLITE_BENCH_ROWS;
    seed_sqlite_tuning_corpus(&sandbox, &passphrase, rows)?;

    let before =
        run_sqlite_latency_child(&sandbox, "read", Some("2000"), Some("full"), "read-before")?;
    let after = run_sqlite_latency_child(&sandbox, "read", None, None, "read-after")?;
    let full_write = run_sqlite_latency_child(&sandbox, "write", None, None, "write-full-default")?;
    let normal_write = run_sqlite_latency_child(
        &sandbox,
        "write",
        None,
        Some("normal"),
        "write-normal-opt-in",
    )?;

    assert_latency_result(&before, -2_000, 2)?;
    assert_latency_result(&after, -65_536, 2)?;
    assert_latency_result(&full_write, -65_536, 2)?;
    assert_latency_result(&normal_write, -65_536, 1)?;
    assert_eq!(
        before.checksum, after.checksum,
        "read variants must scan identical data"
    );

    eprintln!(
        "[SQLiteTuningAB] encrypted=true rows={rows} payload_bytes={SQLITE_BENCH_PAYLOAD_BYTES} \
         read_before(cache_kib=2000,synchronous=FULL):median_ms={:.3},p95_ms={:.3} \
         read_after(cache_kib=65536,synchronous=FULL):median_ms={:.3},p95_ms={:.3} \
         write_default(synchronous=FULL):median_ms={:.3},p95_ms={:.3} \
         write_opt_in(synchronous=NORMAL):median_ms={:.3},p95_ms={:.3}",
        before.median_ms,
        before.p95_ms,
        after.median_ms,
        after.p95_ms,
        full_write.median_ms,
        full_write.p95_ms,
        normal_write.median_ms,
        normal_write.p95_ms
    );
    Ok(())
}

#[test]
fn wal_normal_abrupt_process_exit_recovers_committed_only() -> Result<()> {
    let sandbox = BenchmarkSandbox::new("sqlite-process-exit")?;
    let passphrase = initialize_encrypted_sandbox(&sandbox)?;
    let conn = open_encrypted_benchmark_db(&sandbox, &passphrase)?;
    conn.execute_batch(
        "CREATE TABLE remem_tuning_crash_probe (
             marker TEXT PRIMARY KEY NOT NULL
         );",
    )?;
    drop(conn);

    let committed = run_sqlite_tuning_child(
        &sandbox,
        "crash-committed",
        None,
        Some("normal"),
        None,
        Some("committed-before-exit"),
    )?;
    assert_eq!(
        committed.status.code(),
        Some(91),
        "committed child must terminate without running connection destructors\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&committed.stdout),
        String::from_utf8_lossy(&committed.stderr)
    );

    let uncommitted = run_sqlite_tuning_child(
        &sandbox,
        "crash-uncommitted",
        None,
        Some("normal"),
        None,
        Some("uncommitted-before-exit"),
    )?;
    assert_eq!(
        uncommitted.status.code(),
        Some(92),
        "uncommitted child must terminate without running connection destructors\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&uncommitted.stdout),
        String::from_utf8_lossy(&uncommitted.stderr)
    );

    let recovered = open_encrypted_benchmark_db(&sandbox, &passphrase)?;
    let committed_count: i64 = recovered.query_row(
        "SELECT COUNT(*) FROM remem_tuning_crash_probe WHERE marker = ?1",
        ["committed-before-exit"],
        |row| row.get(0),
    )?;
    let uncommitted_count: i64 = recovered.query_row(
        "SELECT COUNT(*) FROM remem_tuning_crash_probe WHERE marker = ?1",
        ["uncommitted-before-exit"],
        |row| row.get(0),
    )?;
    let integrity: String =
        recovered.pragma_query_value(None, "integrity_check", |row| row.get(0))?;

    assert_eq!(committed_count, 1);
    assert_eq!(uncommitted_count, 0);
    assert_eq!(integrity, "ok");
    Ok(())
}

#[test]
#[ignore = "subprocess helper for SQLite tuning tests"]
fn sqlite_tuning_subprocess_child() -> Result<()> {
    let Some(mode) = std::env::var_os(SQLITE_TUNING_CHILD_ENV) else {
        return Ok(());
    };
    let mode = mode.to_string_lossy();
    let conn = remem::db::open_db_no_migrate()?;

    match mode.as_ref() {
        "read" => write_sqlite_latency_result(&conn, benchmark_sqlite_reads(&conn)?)?,
        "write" => write_sqlite_latency_result(&conn, benchmark_sqlite_writes(&conn)?)?,
        "crash-committed" => {
            ensure_normal_synchronous(&conn)?;
            let marker = required_child_env(SQLITE_TUNING_MARKER_ENV)?;
            conn.execute_batch("BEGIN IMMEDIATE")?;
            conn.execute(
                "INSERT INTO remem_tuning_crash_probe(marker) VALUES (?1)",
                [&marker],
            )?;
            conn.execute_batch("COMMIT")?;
            std::process::exit(91);
        }
        "crash-uncommitted" => {
            ensure_normal_synchronous(&conn)?;
            let marker = required_child_env(SQLITE_TUNING_MARKER_ENV)?;
            conn.execute_batch("BEGIN IMMEDIATE")?;
            conn.execute(
                "INSERT INTO remem_tuning_crash_probe(marker) VALUES (?1)",
                [&marker],
            )?;
            std::process::exit(92);
        }
        unexpected => anyhow::bail!("unexpected SQLite tuning child mode `{unexpected}`"),
    }
    Ok(())
}

#[test]
#[ignore = "large-corpus latency harness; run explicitly with --ignored --nocapture"]
fn query_search_10k_corpus_reports_phase_timings() -> Result<()> {
    let conn = Connection::open_in_memory()?;
    migrate::run_migrations(&conn)?;

    conn.execute("BEGIN IMMEDIATE", [])?;
    for id in 1..=10_000_i64 {
        let title = if id % 500 == 0 {
            format!("FTS5 search latency target {id}")
        } else {
            format!("Noise memory {id}")
        };
        let content = if id % 500 == 0 {
            "FTS5 search should remain measurable on a large in-memory corpus"
        } else {
            "Unrelated memory body for latency benchmark noise"
        };
        conn.execute(
            "INSERT INTO memories
             (id, project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
             VALUES (?1, '/repo', ?2, ?3, 'decision', ?1, ?1, 'active')",
            params![id, title, content],
        )?;
        vector::upsert_memory_embedding(&conn, id, &title, content, "decision", None, "")?;
    }
    conn.execute("COMMIT", [])?;

    let start = std::time::Instant::now();
    let (results, explain) = search::search_with_branch_explain(
        &conn,
        Some("FTS5 search"),
        Some("/repo"),
        None,
        10,
        0,
        true,
        None,
    )?;
    let elapsed = start.elapsed();
    let explain = explain.expect("query search should include explain details");
    eprintln!(
        "[SearchLatency] corpus=10000 returned={} elapsed_ms={} timings=[{}]",
        results.len(),
        elapsed.as_millis(),
        format_phase_timings(&explain.timings)
    );

    assert!(!results.is_empty());
    assert!(explain.timings.iter().any(|timing| timing.phase == "fts"));
    assert!(
        explain
            .timings
            .iter()
            .any(|timing| timing.phase == "vector_load_embeddings"),
        "integrated latency harness must exercise vector embedding load: {:#?}",
        explain.timings
    );
    let vector = explain
        .channels
        .iter()
        .find(|channel| channel.name == "vector" && channel.enabled)
        .expect("integrated latency harness must exercise enabled vector channel");
    assert!(
        vector.candidates_scanned.unwrap_or_default() > 0,
        "integrated latency harness must report vector candidate scan count: {vector:#?}"
    );
    assert!(
        elapsed.as_secs_f64() < 5.0,
        "10k in-memory query search exceeded 5s: {:?}",
        elapsed
    );
    Ok(())
}

#[test]
#[ignore = "large SessionStart staleness harness; run explicitly with --ignored --nocapture"]
fn sessionstart_staleness_large_history_reports_phase() -> Result<()> {
    const OLD_COMMITS: i64 = 50_000;
    const MEMORIES: i64 = 120;
    const POST_ANCHOR_COMMITS: i64 = 256;

    let sandbox = BenchmarkSandbox::new("sessionstart-staleness")?;
    let project = remem::db::project_from_cwd(&sandbox.project_dir.to_string_lossy());

    let initialization = sandbox
        .command()
        .args([
            "search",
            "benchmark-initialization",
            "--project",
            &project,
            "--json",
        ])
        .output()?;
    assert!(
        initialization.status.success(),
        "initialize benchmark database failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&initialization.stdout),
        String::from_utf8_lossy(&initialization.stderr)
    );

    let mut conn = Connection::open(sandbox.data_dir.join("remem.db"))?;
    conn.execute_batch("PRAGMA foreign_keys = ON")?;
    seed_sessionstart_staleness_corpus(
        &mut conn,
        &project,
        OLD_COMMITS,
        MEMORIES,
        POST_ANCHOR_COMMITS,
    )?;
    drop(conn);

    let (output, wall_ms) = sandbox.run_codex_sessionstart(&project)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "SessionStart benchmark failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let hook_output: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse Codex SessionStart hook stdout")?;
    assert_eq!(
        hook_output
            .pointer("/hookSpecificOutput/hookEventName")
            .and_then(serde_json::Value::as_str),
        Some("SessionStart"),
        "benchmark must exercise the Codex SessionStart output adapter:\n{stdout}"
    );
    let additional_context = hook_output
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(serde_json::Value::as_str)
        .context("Codex SessionStart output must contain additionalContext")?;
    assert!(
        additional_context.contains("source_anchor=verify-before-trust"),
        "large-history context must preserve changed-source semantics:\n{additional_context}"
    );
    assert!(
        additional_context.contains("source_anchor=tracked"),
        "large-history context must preserve unchanged-source semantics:\n{additional_context}"
    );
    let done_line = stderr
        .lines()
        .rev()
        .find(|line| line.contains("[context] DONE"))
        .unwrap_or_default();
    assert!(
        done_line.contains(" context_memories=120 "),
        "benchmark must compute context for all {MEMORIES} memories:\n{stderr}"
    );
    assert!(
        done_line.contains("load_staleness_labels="),
        "context DONE log must expose staleness phase:\n{stderr}"
    );
    eprintln!(
        "[SessionStartStaleness] old_commits={OLD_COMMITS} memories={MEMORIES} \
         post_anchor_commits={POST_ANCHOR_COMMITS} wall_ms={wall_ms}\n{done_line}"
    );
    Ok(())
}

fn seed_sessionstart_staleness_corpus(
    conn: &mut Connection,
    project: &str,
    old_commits: i64,
    memory_count: i64,
    post_anchor_commits: i64,
) -> Result<()> {
    let transaction = conn.transaction()?;
    {
        let mut insert_commit = transaction.prepare(
            "INSERT INTO git_commits(
                 id, project, repo_path, sha, short_sha, branch, changed_files,
                 authored_at_epoch, created_at_epoch, updated_at_epoch
             ) VALUES (?1, ?2, ?2, ?3, ?3, 'main', ?4, ?5, ?5, ?5)",
        )?;
        for id in 1..=old_commits {
            insert_commit.execute(params![id, project, format!("old-{id}"), "[]", id])?;
        }

        let source_epoch = old_commits + 10_000;
        for memory_id in 1..=memory_count {
            let commit_id = old_commits + memory_id;
            let path = format!("src/bench_{memory_id}.rs");
            insert_commit.execute(params![
                commit_id,
                project,
                format!("source-{memory_id}"),
                serde_json::to_string(&[&path])?,
                source_epoch
            ])?;
            transaction.execute(
                "INSERT INTO git_commit_sessions(
                     commit_id, session_id, memory_session_id, source, linked_at_epoch
                 ) VALUES (?1, ?2, ?2, 'benchmark', ?3)",
                params![
                    commit_id,
                    format!("benchmark-memory-{memory_id}"),
                    source_epoch
                ],
            )?;
            transaction.execute(
                "INSERT INTO memories(
                     id, session_id, project, topic_key, title, content, memory_type,
                     files, created_at_epoch, updated_at_epoch, status, branch, scope
                 ) VALUES (
                     ?1, ?2, ?3, ?4, ?5, ?6, 'decision', ?7,
                     ?8, ?8, 'active', 'main', 'project'
                 )",
                params![
                    memory_id,
                    format!("benchmark-memory-{memory_id}"),
                    project,
                    format!("project-decision-{memory_id}"),
                    format!("Project decision {memory_id}"),
                    format!("Unique implementation constraint number {memory_id}."),
                    serde_json::to_string(&[&path])?,
                    source_epoch
                ],
            )?;
        }

        for offset in 1..=post_anchor_commits {
            let id = old_commits + memory_count + offset;
            let path = if offset == post_anchor_commits {
                "src/bench_1.rs".to_string()
            } else {
                format!("src/post_noise_{offset}.rs")
            };
            insert_commit.execute(params![
                id,
                project,
                format!("post-{offset}"),
                serde_json::to_string(&[path])?,
                source_epoch + offset
            ])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct SqliteLatencyResult {
    median_ms: f64,
    p95_ms: f64,
    cache_size: i64,
    synchronous: i64,
    temp_store: i64,
    checksum: i64,
}

fn initialize_encrypted_sandbox(sandbox: &BenchmarkSandbox) -> Result<String> {
    let passphrase = format!(
        "{:x}",
        Sha256::digest(sandbox.root.to_string_lossy().as_bytes())
    );
    std::fs::write(sandbox.data_dir.join(".key"), &passphrase)?;

    let project = remem::db::project_from_cwd(&sandbox.project_dir.to_string_lossy());
    let output = sandbox
        .command()
        .args([
            "search",
            "sqlite-benchmark-initialization",
            "--project",
            &project,
            "--json",
        ])
        .output()?;
    ensure!(
        output.status.success(),
        "initialize encrypted benchmark database failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let raw = Connection::open(sandbox.data_dir.join("remem.db"))?;
    let raw_schema = raw.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| {
        row.get::<_, i64>(0)
    });
    ensure!(
        raw_schema.is_err(),
        "benchmark database must be unreadable without its SQLCipher key"
    );

    let keyed = open_encrypted_benchmark_db(sandbox, &passphrase)?;
    let cipher_version: String =
        keyed.pragma_query_value(None, "cipher_version", |row| row.get(0))?;
    ensure!(
        !cipher_version.trim().is_empty(),
        "benchmark connection must report a SQLCipher version"
    );
    Ok(passphrase)
}

fn open_encrypted_benchmark_db(sandbox: &BenchmarkSandbox, passphrase: &str) -> Result<Connection> {
    let conn = Connection::open(sandbox.data_dir.join("remem.db"))?;
    conn.pragma_update(None, "key", passphrase)?;
    conn.query_row("SELECT COUNT(*) FROM sqlite_schema", [], |row| {
        row.get::<_, i64>(0)
    })
    .context("open encrypted benchmark database with generated key")?;
    Ok(conn)
}

fn seed_sqlite_tuning_corpus(
    sandbox: &BenchmarkSandbox,
    passphrase: &str,
    rows: usize,
) -> Result<()> {
    let mut conn = open_encrypted_benchmark_db(sandbox, passphrase)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         CREATE TABLE remem_tuning_read_benchmark (
             id INTEGER PRIMARY KEY,
             payload BLOB NOT NULL
         );
         CREATE TABLE remem_tuning_write_benchmark (
             id INTEGER PRIMARY KEY,
             marker TEXT NOT NULL
         );",
    )?;
    let transaction = conn.transaction()?;
    {
        let payload = vec![b'x'; SQLITE_BENCH_PAYLOAD_BYTES];
        let mut insert = transaction
            .prepare("INSERT INTO remem_tuning_read_benchmark(id, payload) VALUES (?1, ?2)")?;
        for id in 1..=rows {
            insert.execute(params![id as i64, &payload])?;
        }
    }
    transaction.commit()?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    Ok(())
}

fn run_sqlite_latency_child(
    sandbox: &BenchmarkSandbox,
    mode: &str,
    cache_kib: Option<&str>,
    synchronous: Option<&str>,
    result_name: &str,
) -> Result<SqliteLatencyResult> {
    let result_path = sandbox.root.join(format!("{result_name}.json"));
    let output = run_sqlite_tuning_child(
        sandbox,
        mode,
        cache_kib,
        synchronous,
        Some(&result_path),
        None,
    )?;
    ensure!(
        output.status.success(),
        "SQLite tuning {result_name} child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = std::fs::read(&result_path)
        .with_context(|| format!("read SQLite tuning result {}", result_path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parse SQLite tuning result {}", result_path.display()))
}

fn run_sqlite_tuning_child(
    sandbox: &BenchmarkSandbox,
    mode: &str,
    cache_kib: Option<&str>,
    synchronous: Option<&str>,
    result_path: Option<&Path>,
    marker: Option<&str>,
) -> Result<Output> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args([
            "--exact",
            "sqlite_tuning_subprocess_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .current_dir(&sandbox.project_dir)
        .env("HOME", sandbox.root.join("home"))
        .env("XDG_CONFIG_HOME", sandbox.root.join("xdg-config"))
        .env("XDG_CACHE_HOME", sandbox.root.join("xdg-cache"))
        .env("REMEM_DATA_DIR", &sandbox.data_dir)
        .env("REMEM_CONFIG", sandbox.root.join("config.toml"))
        .env("REMEM_EMBEDDINGS_PROVIDER", "off")
        .env("REMEM_RERANK_ENABLED", "0")
        .env(SQLITE_TUNING_CHILD_ENV, mode)
        .env_remove("REMEM_CIPHER_KEY")
        .env_remove(SQLITE_CACHE_KIB_ENV)
        .env_remove(SQLITE_SYNCHRONOUS_ENV)
        .env_remove(SQLITE_TUNING_RESULT_ENV)
        .env_remove(SQLITE_TUNING_MARKER_ENV);
    if let Some(value) = cache_kib {
        command.env(SQLITE_CACHE_KIB_ENV, value);
    }
    if let Some(value) = synchronous {
        command.env(SQLITE_SYNCHRONOUS_ENV, value);
    }
    if let Some(path) = result_path {
        command.env(SQLITE_TUNING_RESULT_ENV, path);
    }
    if let Some(value) = marker {
        command.env(SQLITE_TUNING_MARKER_ENV, value);
    }
    command.output().context("run SQLite tuning test child")
}

fn benchmark_sqlite_reads(conn: &Connection) -> Result<(Vec<f64>, i64)> {
    let query = "SELECT SUM(length(payload)) FROM remem_tuning_read_benchmark";
    let expected: i64 = conn.query_row(query, [], |row| row.get(0))?;
    let mut samples_ms = Vec::with_capacity(9);
    for _ in 0..9 {
        let started = Instant::now();
        let checksum: i64 = conn.query_row(query, [], |row| row.get(0))?;
        ensure!(checksum == expected, "SQLite read benchmark checksum drift");
        samples_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    Ok((samples_ms, expected))
}

fn benchmark_sqlite_writes(conn: &Connection) -> Result<(Vec<f64>, i64)> {
    conn.execute(
        "INSERT INTO remem_tuning_write_benchmark(marker) VALUES (?1)",
        [format!("{}-warmup", std::process::id())],
    )?;
    let mut samples_ms = Vec::with_capacity(31);
    for sample in 0..31 {
        let started = Instant::now();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        conn.execute(
            "INSERT INTO remem_tuning_write_benchmark(marker) VALUES (?1)",
            [format!("{}-{sample}", std::process::id())],
        )?;
        conn.execute_batch("COMMIT")?;
        samples_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let count = conn.query_row(
        "SELECT COUNT(*) FROM remem_tuning_write_benchmark",
        [],
        |row| row.get(0),
    )?;
    Ok((samples_ms, count))
}

fn write_sqlite_latency_result(
    conn: &Connection,
    (samples_ms, checksum): (Vec<f64>, i64),
) -> Result<()> {
    let result_path = required_child_env(SQLITE_TUNING_RESULT_ENV)?;
    let mut sorted = samples_ms.clone();
    sorted.sort_by(f64::total_cmp);
    let median_ms = sorted[sorted.len() / 2];
    let p95_index = ((sorted.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    let result = serde_json::json!({
        "median_ms": median_ms,
        "p95_ms": sorted[p95_index],
        "cache_size": pragma_i64(conn, "cache_size")?,
        "synchronous": pragma_i64(conn, "synchronous")?,
        "temp_store": pragma_i64(conn, "temp_store")?,
        "checksum": checksum,
    });
    std::fs::write(&result_path, serde_json::to_vec_pretty(&result)?)
        .with_context(|| format!("write SQLite tuning result {result_path}"))?;
    Ok(())
}

fn assert_latency_result(
    result: &SqliteLatencyResult,
    cache_size: i64,
    synchronous: i64,
) -> Result<()> {
    ensure!(
        result.median_ms.is_finite() && result.p95_ms.is_finite(),
        "benchmark latencies must be finite"
    );
    ensure!(result.cache_size == cache_size, "cache_size mismatch");
    ensure!(result.synchronous == synchronous, "synchronous mismatch");
    ensure!(result.temp_store == 2, "temp_store must be MEMORY");
    Ok(())
}

fn ensure_normal_synchronous(conn: &Connection) -> Result<()> {
    let journal_mode: String = conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
    ensure!(
        journal_mode.eq_ignore_ascii_case("wal"),
        "crash child must use journal_mode=WAL"
    );
    ensure!(
        pragma_i64(conn, "synchronous")? == 1,
        "crash child must use synchronous=NORMAL"
    );
    Ok(())
}

fn pragma_i64(conn: &Connection, name: &str) -> Result<i64> {
    conn.pragma_query_value(None, name, |row| row.get(0))
        .with_context(|| format!("read PRAGMA {name}"))
}

fn required_child_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} must be set for SQLite tuning child"))
}

struct BenchmarkSandbox {
    root: PathBuf,
    data_dir: PathBuf,
    project_dir: PathBuf,
}

impl BenchmarkSandbox {
    fn new(label: &str) -> Result<Self> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("remem-{label}-{}-{nonce}", std::process::id()));
        let data_dir = root.join("data");
        let project_dir = root.join("project");
        std::fs::create_dir_all(root.join("home"))?;
        std::fs::create_dir_all(root.join("xdg-config"))?;
        std::fs::create_dir_all(root.join("xdg-cache"))?;
        std::fs::create_dir_all(&data_dir)?;
        std::fs::create_dir_all(&project_dir)?;
        Ok(Self {
            root,
            data_dir,
            project_dir,
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_remem"));
        command
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("xdg-config"))
            .env("XDG_CACHE_HOME", self.root.join("xdg-cache"))
            .env("REMEM_DATA_DIR", &self.data_dir)
            .env("REMEM_CONFIG", self.root.join("config.toml"))
            .env("REMEM_ALLOW_PLAINTEXT_DB", "1")
            .env("REMEM_EMBEDDINGS_PROVIDER", "off")
            .env("REMEM_RERANK_ENABLED", "0")
            .env("REMEM_CONTEXT_DEBUG", "0")
            .env("REMEM_CONTEXT_TOTAL_CHAR_LIMIT", "12000")
            .env("REMEM_CONTEXT_CANDIDATE_FETCH_LIMIT", "120")
            .env("REMEM_CONTEXT_MEMORY_INDEX_LIMIT", "50")
            .env("REMEM_CONTEXT_MEMORY_INDEX_CHAR_LIMIT", "4000")
            .env("REMEM_CONTEXT_CORE_ITEM_LIMIT", "6")
            .env("REMEM_CONTEXT_CORE_CHAR_LIMIT", "3000")
            .env("REMEM_CONTEXT_SESSION_COUNT", "5")
            .env("REMEM_CONTEXT_SELF_DIAGNOSTIC_LIMIT", "2")
            .env("REMEM_CONTEXT_PREFERENCE_PROJECT_LIMIT", "20")
            .env("REMEM_CONTEXT_PREFERENCE_GLOBAL_LIMIT", "5")
            .env("REMEM_CONTEXT_PREFERENCE_CHAR_LIMIT", "1500")
            .env("REMEM_CONTEXT_LESSON_LIMIT", "4")
            .env("REMEM_CONTEXT_LESSON_CHAR_LIMIT", "1200")
            .env("REMEM_CONTEXT_RELEVANCE_K", "1")
            .env("NO_COLOR", "1")
            .env_remove("REMEM_CIPHER_KEY")
            .env_remove(SQLITE_CACHE_KIB_ENV)
            .env_remove(SQLITE_SYNCHRONOUS_ENV)
            .env_remove("REMEM_CONTEXT_OBSERVATIONS")
            .env_remove("REMEM_DISABLE_HOOKS")
            .env_remove("REMEM_STDERR_TO_LOG")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE");
        command
    }

    fn run_codex_sessionstart(&self, project: &str) -> Result<(Output, u128)> {
        let payload = serde_json::json!({
            "session_id": "benchmark-sessionstart",
            "cwd": project,
            "target": {
                "type": "SessionStart",
                "source": "Startup"
            }
        });
        let started = Instant::now();
        let mut child = self
            .command()
            .current_dir(&self.project_dir)
            .args(["context", "--host", "codex-cli", "--gate", "off", "--force"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .context("open SessionStart benchmark stdin")?
            .write_all(serde_json::to_string(&payload)?.as_bytes())?;
        let output = child.wait_with_output()?;
        Ok((output, started.elapsed().as_millis()))
    }
}

impl Drop for BenchmarkSandbox {
    fn drop(&mut self) {
        if let Err(error) = remove_benchmark_dir(&self.root) {
            eprintln!(
                "failed to remove benchmark sandbox {}: {error}",
                self.root.display()
            );
        }
    }
}

fn remove_benchmark_dir(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}
