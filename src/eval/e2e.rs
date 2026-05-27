use std::ffi::OsString;
use std::fmt::{self, Display};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{oneshot, Mutex};

const PROJECT: &str = "/remem/eval-e2e";
const CORPUS_NAME: &str = "builtin-coding-agent-life-v1";

#[derive(Debug, Clone, Copy)]
pub struct E2eEvalOptions {
    pub k: usize,
    pub keep_data_dir: bool,
}

impl Default for E2eEvalOptions {
    fn default() -> Self {
        Self {
            k: 5,
            keep_data_dir: false,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct E2eEvalReport {
    pub metadata: E2eEvalMetadata,
    pub api_metrics: E2eMetricSummary,
    pub keyword_baseline: E2eMetricSummary,
    pub queries: Vec<E2eQueryReport>,
}

#[derive(Debug, Serialize)]
pub struct E2eEvalMetadata {
    pub commit: Option<String>,
    pub command: String,
    pub corpus: String,
    pub corpus_items: usize,
    pub query_count: usize,
    pub data_dir: String,
    pub data_dir_kept: bool,
    pub api_base_url: String,
    pub config: E2eEvalConfig,
}

#[derive(Debug, Serialize)]
pub struct E2eEvalConfig {
    pub boundary: String,
    pub project: String,
    pub k: usize,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct E2eMetricSummary {
    pub total_queries: usize,
    pub hit_count: usize,
    pub hit_rate: f64,
    pub mrr: f64,
}

#[derive(Debug, Serialize)]
pub struct E2eQueryReport {
    pub id: String,
    pub query: String,
    pub expected_topic_key: String,
    pub api_rank: Option<usize>,
    pub keyword_baseline_rank: Option<usize>,
    pub api_result_topic_keys: Vec<String>,
    pub keyword_baseline_topic_keys: Vec<String>,
}

#[derive(Clone, Copy)]
struct CorpusMemory {
    topic_key: &'static str,
    title: &'static str,
    text: &'static str,
    memory_type: &'static str,
}

#[derive(Clone, Copy)]
struct EvalQuery {
    id: &'static str,
    query: &'static str,
    expected_topic_key: &'static str,
}

const CORPUS: &[CorpusMemory] = &[
    CorpusMemory {
        topic_key: "eval-migration-locking",
        title: "SQLite migration locking fix",
        text: "Root cause: concurrent schema migrations raced on startup. Fix: serialize migration execution and verify with cargo test migrate::tests.",
        memory_type: "bugfix",
    },
    CorpusMemory {
        topic_key: "eval-raw-archive-fallback",
        title: "Raw archive fallback for sparse recall",
        text: "When curated search is sparse, remem should attach raw archive hits so literal chat content remains discoverable without treating raw rows as curated memories.",
        memory_type: "decision",
    },
    CorpusMemory {
        topic_key: "eval-codex-hook-timeout",
        title: "Codex hook stdin timeout",
        text: "Codex hook stdin reads must allow normal startup latency and then fall back to CLI values instead of failing the whole capture path.",
        memory_type: "discovery",
    },
    CorpusMemory {
        topic_key: "eval-project-scope-discipline",
        title: "Project scope discipline",
        text: "Global memories must be explicitly requested; project memories should not leak into unrelated workspaces during context injection or retrieval.",
        memory_type: "preference",
    },
];

const QUERIES: &[EvalQuery] = &[
    EvalQuery {
        id: "migration-race",
        query: "schema migration race serialize startup",
        expected_topic_key: "eval-migration-locking",
    },
    EvalQuery {
        id: "raw-fallback",
        query: "sparse curated search raw archive fallback",
        expected_topic_key: "eval-raw-archive-fallback",
    },
    EvalQuery {
        id: "codex-timeout",
        query: "Codex hook stdin timeout fallback CLI values",
        expected_topic_key: "eval-codex-hook-timeout",
    },
    EvalQuery {
        id: "scope-leak",
        query: "global memories explicit project leak context retrieval",
        expected_topic_key: "eval-project-scope-discipline",
    },
];

#[derive(Serialize)]
struct ApiSaveRequest<'a> {
    text: &'a str,
    title: &'a str,
    project: &'a str,
    topic_key: &'a str,
    memory_type: &'a str,
    scope: &'a str,
    local_copy_enabled: bool,
}

#[derive(Deserialize)]
struct ApiSaveResponse {
    id: i64,
}

#[derive(Deserialize)]
struct ApiSearchResponse {
    data: Vec<ApiMemoryItem>,
}

#[derive(Deserialize)]
struct ApiMemoryItem {
    topic_key: Option<String>,
}

pub async fn run_sandbox_eval(options: E2eEvalOptions) -> Result<E2eEvalReport> {
    let _env_guard = env_lock().lock().await;
    let k = options.k.max(1);
    let data_dir = unique_temp_data_dir();
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("create eval data dir {}", data_dir.display()))?;
    let _restore = EnvRestore::set("REMEM_DATA_DIR", data_dir.as_os_str().to_os_string());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind sandbox eval API listener")?;
    let addr = listener
        .local_addr()
        .context("read sandbox API listener addr")?;
    let base_url = format!("http://{}", addr);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let app = crate::api::build_router(addr.port()).with_state(crate::api::DbState);
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    let client = reqwest::Client::new();
    let run_result = run_api_boundary_eval(&client, &base_url, k).await;
    let _ = shutdown_tx.send(());
    let server_result = server.await.context("join sandbox eval API server")?;
    server_result.context("sandbox eval API server failed")?;

    let mut report = run_result?;
    report.metadata.commit = current_git_commit();
    report.metadata.command = format!("remem eval-e2e --k {}", k);
    report.metadata.data_dir = data_dir.display().to_string();
    report.metadata.data_dir_kept = options.keep_data_dir;
    report.metadata.api_base_url = base_url;

    if !options.keep_data_dir {
        std::fs::remove_dir_all(&data_dir)
            .with_context(|| format!("remove eval data dir {}", data_dir.display()))?;
    }
    Ok(report)
}

async fn run_api_boundary_eval(
    client: &reqwest::Client,
    base_url: &str,
    k: usize,
) -> Result<E2eEvalReport> {
    wait_for_status(client, base_url).await?;
    let mut saved_ids = Vec::with_capacity(CORPUS.len());
    for memory in CORPUS {
        let saved = save_memory_via_api(client, base_url, memory).await?;
        saved_ids.push(saved.id);
    }

    let mut query_reports = Vec::with_capacity(QUERIES.len());
    for query in QUERIES {
        let api_topic_keys = search_topic_keys_via_api(client, base_url, query.query, k).await?;
        let keyword_topic_keys = keyword_baseline_topic_keys(query.query, k);
        query_reports.push(E2eQueryReport {
            id: query.id.to_string(),
            query: query.query.to_string(),
            expected_topic_key: query.expected_topic_key.to_string(),
            api_rank: one_based_rank(&api_topic_keys, query.expected_topic_key),
            keyword_baseline_rank: one_based_rank(&keyword_topic_keys, query.expected_topic_key),
            api_result_topic_keys: api_topic_keys,
            keyword_baseline_topic_keys: keyword_topic_keys,
        });
    }

    Ok(E2eEvalReport {
        metadata: E2eEvalMetadata {
            commit: None,
            command: String::new(),
            corpus: CORPUS_NAME.to_string(),
            corpus_items: saved_ids.len(),
            query_count: QUERIES.len(),
            data_dir: String::new(),
            data_dir_kept: false,
            api_base_url: String::new(),
            config: E2eEvalConfig {
                boundary: "REST API /api/v1/memories + /api/v1/search".to_string(),
                project: PROJECT.to_string(),
                k,
            },
        },
        api_metrics: summarize_ranks(query_reports.iter().map(|query| query.api_rank)),
        keyword_baseline: summarize_ranks(
            query_reports
                .iter()
                .map(|query| query.keyword_baseline_rank),
        ),
        queries: query_reports,
    })
}

async fn wait_for_status(client: &reqwest::Client, base_url: &str) -> Result<()> {
    let url = format!("{}/api/v1/status", base_url);
    let mut last_error = None;
    for _ in 0..20 {
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => return Ok(()),
            Ok(response) => last_error = Some(anyhow!("status returned {}", response.status())),
            Err(error) => last_error = Some(error.into()),
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Err(last_error.unwrap_or_else(|| anyhow!("status endpoint did not respond")))
}

async fn save_memory_via_api(
    client: &reqwest::Client,
    base_url: &str,
    memory: &CorpusMemory,
) -> Result<ApiSaveResponse> {
    let request = ApiSaveRequest {
        text: memory.text,
        title: memory.title,
        project: PROJECT,
        topic_key: memory.topic_key,
        memory_type: memory.memory_type,
        scope: "project",
        local_copy_enabled: false,
    };
    let response = client
        .post(format!("{}/api/v1/memories", base_url))
        .json(&request)
        .send()
        .await
        .context("POST /api/v1/memories failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("POST /api/v1/memories returned {status}: {body}"));
    }
    response
        .json::<ApiSaveResponse>()
        .await
        .context("parse save memory response")
}

async fn search_topic_keys_via_api(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    k: usize,
) -> Result<Vec<String>> {
    let limit = k.to_string();
    let response = client
        .get(format!("{}/api/v1/search", base_url))
        .query(&[
            ("query", query),
            ("project", PROJECT),
            ("limit", limit.as_str()),
        ])
        .send()
        .await
        .context("GET /api/v1/search failed")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("GET /api/v1/search returned {status}: {body}"));
    }
    let search = response
        .json::<ApiSearchResponse>()
        .await
        .context("parse search response")?;
    Ok(search
        .data
        .into_iter()
        .filter_map(|item| item.topic_key)
        .collect())
}

fn keyword_baseline_topic_keys(query: &str, k: usize) -> Vec<String> {
    let query_tokens = tokenize(query);
    let mut scored: Vec<(usize, &'static str)> = CORPUS
        .iter()
        .map(|memory| {
            let text = format!("{} {}", memory.title, memory.text);
            let doc_tokens = tokenize(&text);
            let score = query_tokens
                .iter()
                .filter(|token| doc_tokens.contains(*token))
                .count();
            (score, memory.topic_key)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(k)
        .map(|(_, topic_key)| topic_key.to_string())
        .collect()
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = text
        .to_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() > 2)
        .map(str::to_string)
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens
}

fn one_based_rank(topic_keys: &[String], expected_topic_key: &str) -> Option<usize> {
    topic_keys
        .iter()
        .position(|topic_key| topic_key == expected_topic_key)
        .map(|index| index + 1)
}

fn summarize_ranks(ranks: impl Iterator<Item = Option<usize>>) -> E2eMetricSummary {
    let mut total_queries = 0usize;
    let mut hit_count = 0usize;
    let mut reciprocal_sum = 0.0;
    for rank in ranks {
        total_queries += 1;
        if let Some(rank) = rank {
            hit_count += 1;
            reciprocal_sum += 1.0 / rank as f64;
        }
    }
    E2eMetricSummary {
        total_queries,
        hit_count,
        hit_rate: if total_queries == 0 {
            0.0
        } else {
            hit_count as f64 / total_queries as f64
        },
        mrr: if total_queries == 0 {
            0.0
        } else {
            reciprocal_sum / total_queries as f64
        },
    }
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvRestore {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: OsString) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn unique_temp_data_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("remem-e2e-eval-{}-{}", std::process::id(), nanos))
}

fn current_git_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

impl Display for E2eEvalReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "=== remem eval-e2e ({}, {} queries, k={}) ===",
            self.metadata.corpus, self.metadata.query_count, self.metadata.config.k
        )?;
        writeln!(f, "boundary: {}", self.metadata.config.boundary)?;
        writeln!(f, "data_dir: {}", self.metadata.data_dir)?;
        writeln!(f, "data_dir_kept: {}", self.metadata.data_dir_kept)?;
        writeln!(
            f,
            "api: hit_rate={:.1}% mrr={:.3} ({}/{})",
            self.api_metrics.hit_rate * 100.0,
            self.api_metrics.mrr,
            self.api_metrics.hit_count,
            self.api_metrics.total_queries
        )?;
        writeln!(
            f,
            "keyword_baseline: hit_rate={:.1}% mrr={:.3} ({}/{})",
            self.keyword_baseline.hit_rate * 100.0,
            self.keyword_baseline.mrr,
            self.keyword_baseline.hit_count,
            self.keyword_baseline.total_queries
        )?;
        for query in &self.queries {
            writeln!(
                f,
                "- {} api_rank={:?} baseline_rank={:?}",
                query.id, query.api_rank, query.keyword_baseline_rank
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_ranks_reports_hit_rate_and_mrr() {
        let got = summarize_ranks([Some(1), Some(4), None].into_iter());
        assert_eq!(
            got,
            E2eMetricSummary {
                total_queries: 3,
                hit_count: 2,
                hit_rate: 2.0 / 3.0,
                mrr: (1.0 + 0.25) / 3.0,
            }
        );
    }

    #[test]
    fn keyword_baseline_ranks_expected_memory() {
        let got = keyword_baseline_topic_keys("raw archive fallback sparse search", 2);
        assert_eq!(
            got.first().map(String::as_str),
            Some("eval-raw-archive-fallback")
        );
    }
}
