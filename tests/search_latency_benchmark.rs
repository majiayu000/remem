use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Instant;

use remem::{
    migrate,
    perf::format_phase_timings,
    retrieval::{search, vector},
};

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
