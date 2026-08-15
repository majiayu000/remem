use std::ffi::OsString;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::audit_contract::RememContextAuditSnapshot;
use super::capture_projection::{
    build_capture_plan, new_opaque_id, write_capture_plan, CaptureWriteTrace, FIXED_PROJECT_PATH,
};
use super::condition::{ScopedEnvVars, BENCHMARK_CONTEXT_ENV_OVERRIDES};
use super::types::{CodingBenchTask, CodingMemoryAttributionInput};

const LEASE_OWNER: &str = "coding-bench-remem-e2e";
const LEASE_SECS: i64 = 480;
const TASK_TIMEOUT_SECS: u64 = 420;
const MAX_DRAIN_TASKS: usize = 16;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct E2ePipelineTrace {
    pub projection_schema: String,
    pub projection_sha256: String,
    pub call_plan_sha256: String,
    pub memory_config_sha256: String,
    pub captured_count: usize,
    pub extracted_observation_count: usize,
    pub candidate_count: usize,
    pub promoted_memory_count: usize,
    pub retrieved_memory_count: usize,
    pub drained_task_count: usize,
    pub pipeline_starved: bool,
    pub sessionstart_sha256: String,
}

pub(super) struct PreparedE2e {
    pub rendered_context: String,
    pub memory_attribution: CodingMemoryAttributionInput,
    pub context_audit: RememContextAuditSnapshot,
    pub trace: E2ePipelineTrace,
}

pub(super) async fn prepare_remem_e2e(
    data_dir: &Path,
    task: &CodingBenchTask,
    memory_config: &Path,
) -> Result<PreparedE2e> {
    validate_memory_config(memory_config)?;
    let memory_config_sha256 = format!(
        "{:x}",
        Sha256::digest(fs::read(memory_config).with_context(|| format!(
            "read remem_e2e memory config {}",
            memory_config.display()
        ))?)
    );
    let _environment_lock = crate::runtime_config::ENV_LOCK.lock().map_err(|error| {
        anyhow::anyhow!("acquire benchmark environment lock before remem_e2e: {error}")
    })?;
    fs::create_dir_all(data_dir).context("create remem_e2e data directory")?;
    let _context_environment = ScopedEnvVars::remove_many(BENCHMARK_CONTEXT_ENV_OVERRIDES);
    let _environment = ScopedEnvVars::set_many([
        ("REMEM_DATA_DIR", data_dir.as_os_str().to_os_string()),
        ("REMEM_ALLOW_PLAINTEXT_DB", OsString::from("1")),
        ("REMEM_CONTEXT_BUNDLE_RENDER_MODE", OsString::from("bundle")),
        ("REMEM_CONTEXT_GATE_HOSTS", OsString::from("codex-cli")),
        ("REMEM_CONFIG", memory_config.as_os_str().to_os_string()),
    ]);
    crate::runtime_config::resolve_memory_ai_profile(crate::runtime_config::MemoryAiSelection {
        host: Some("codex-cli"),
        profile: None,
    })
    .context("resolve explicit remem_e2e memory-AI profile")?;

    let plan = build_capture_plan(task)?;
    let session_id = new_opaque_id("e2e-")?;
    let conn = crate::db::open_db().context("open isolated remem_e2e database")?;
    let capture = write_capture_plan(&conn, &plan, &session_id)?;
    drop(conn);

    let drained_task_count = drain_production_tasks().await?;
    let conn = crate::db::open_db().context("reopen drained remem_e2e database")?;
    validate_drain_closed(&conn)?;
    let extracted_observation_count = query_trace_row_count(&conn, "observations")?;
    let candidate_count = query_trace_row_count(&conn, "memory_candidates")?;
    let promoted_memory_ids = query_active_memory_ids(&conn)?;

    let emission = crate::context::session_start_benchmark_emission(
        FIXED_PROJECT_PATH,
        FIXED_PROJECT_PATH,
        "codex-cli",
    )
    .context("render remem_e2e production SessionStart context")?;
    let injection_run_id = emission
        .injection_run_id
        .as_deref()
        .context("remem_e2e SessionStart omitted persisted injection_run_id")?;
    let context_audit =
        super::audit_contract::load_context_audit_snapshot(&conn, injection_run_id)?
            .context("remem_e2e SessionStart omitted persisted ContextAudit")?;
    super::audit_contract::verify_snapshot_against_persisted_injection(
        &conn,
        &context_audit,
        &emission.rendered_output,
    )
    .context("verify remem_e2e persisted SessionStart ContextAudit")?;
    let injected_memory_ids = injected_memory_ids(&conn, injection_run_id)?;
    let trace = build_trace(
        &plan,
        capture,
        extracted_observation_count,
        candidate_count,
        promoted_memory_ids.len(),
        injected_memory_ids.len(),
        drained_task_count,
        &emission.rendered_output,
        &memory_config_sha256,
    );
    Ok(PreparedE2e {
        rendered_context: emission.rendered_output,
        memory_attribution: CodingMemoryAttributionInput {
            injected_memory_ids,
            relevant_memory_ids: promoted_memory_ids,
            forbidden_memory_ids: Vec::new(),
            gold_required_facts: task.gold_memory.required_facts.clone(),
            gold_forbidden_facts: task.gold_memory.forbidden_facts.clone(),
        },
        context_audit,
        trace,
    })
}

async fn drain_production_tasks() -> Result<usize> {
    let mut drained = 0;
    while drained < MAX_DRAIN_TASKS {
        if !crate::extraction_worker::run_next(LEASE_OWNER, LEASE_SECS, TASK_TIMEOUT_SECS).await? {
            return Ok(drained);
        }
        drained += 1;
    }
    bail!("remem_e2e production drain exceeded {MAX_DRAIN_TASKS} tasks")
}

fn validate_drain_closed(conn: &Connection) -> Result<()> {
    let unexpected: i64 = conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks
         WHERE task_kind NOT IN ('observation_extract', 'memory_candidate', 'graph_candidate')",
        [],
        |row| row.get(0),
    )?;
    if unexpected != 0 {
        bail!("remem_e2e production drain created an unexpected extraction task kind");
    }
    let residual: i64 = conn.query_row(
        "SELECT COUNT(*) FROM extraction_tasks WHERE status != 'done'",
        [],
        |row| row.get(0),
    )?;
    if residual != 0 {
        bail!("remem_e2e production drain left {residual} residual/failed tasks");
    }
    Ok(())
}

fn query_active_memory_ids(conn: &Connection) -> Result<Vec<i64>> {
    let mut statement = conn.prepare(
        "SELECT id FROM memories
         WHERE project = ?1 AND status = 'active'
         ORDER BY id ASC",
    )?;
    let rows = statement.query_map([FIXED_PROJECT_PATH], |row| row.get::<_, i64>(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn injected_memory_ids(conn: &Connection, injection_run_id: &str) -> Result<Vec<i64>> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT memory_id FROM context_injection_items
         WHERE injection_run_id = ?1 AND status = 'injected' AND memory_id IS NOT NULL
         ORDER BY memory_id ASC",
    )?;
    let rows = statement.query_map([injection_run_id], |row| row.get::<_, i64>(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn query_trace_row_count(conn: &Connection, table: &str) -> Result<usize> {
    let sql = match table {
        "observations" => "SELECT COUNT(*) FROM observations",
        "memory_candidates" => "SELECT COUNT(*) FROM memory_candidates",
        _ => bail!("unsupported remem_e2e trace table"),
    };
    let count: i64 = conn.query_row(sql, [], |row| row.get(0))?;
    usize::try_from(count).context("convert remem_e2e trace count")
}

#[allow(clippy::too_many_arguments)]
fn build_trace(
    plan: &super::capture_projection::CapturePlan,
    capture: CaptureWriteTrace,
    extracted_observation_count: usize,
    candidate_count: usize,
    promoted_memory_count: usize,
    retrieved_memory_count: usize,
    drained_task_count: usize,
    rendered_context: &str,
    memory_config_sha256: &str,
) -> E2ePipelineTrace {
    E2ePipelineTrace {
        projection_schema: super::capture_projection::PROJECTION_SCHEMA.to_string(),
        projection_sha256: plan.projection_sha256.clone(),
        call_plan_sha256: plan.call_plan_sha256.clone(),
        memory_config_sha256: memory_config_sha256.to_string(),
        captured_count: capture.captured_count,
        extracted_observation_count,
        candidate_count,
        promoted_memory_count,
        retrieved_memory_count,
        drained_task_count,
        pipeline_starved: promoted_memory_count == 0,
        sessionstart_sha256: format!("{:x}", Sha256::digest(rendered_context.as_bytes())),
    }
}

fn reject_config_symlink(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect remem_e2e memory config {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("remem_e2e memory config must be a regular non-symlink file");
    }
    Ok(())
}

pub(super) fn validate_memory_config(path: &Path) -> Result<()> {
    reject_config_symlink(path)?;
    let text = fs::read_to_string(path)
        .with_context(|| format!("read remem_e2e memory config {}", path.display()))?;
    if text.trim().is_empty() {
        bail!("remem_e2e memory config must not be empty");
    }
    text.parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parse remem_e2e memory config {}", path.display()))?;
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[tokio::test]
    async fn production_pipeline_reaches_audited_sessionstart() -> Result<()> {
        let root = std::env::temp_dir().join(format!(
            "remem-coding-e2e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir_all(&root)?;
        let stub = root.join("codex-stub.sh");
        fs::write(
            &stub,
            r#"#!/bin/sh
prev=""
output_path=""
for arg in "$@"; do
  [ "$prev" = "--output-last-message" ] && { output_path="$arg"; break; }
  prev="$arg"
done
[ -n "$output_path" ] || exit 1
input_path="${TMPDIR:-/tmp}/remem-coding-e2e-$$.txt"
trap 'rm -f "$input_path"' EXIT
cat > "$input_path"
if grep -q "Task: memory_candidate" "$input_path"; then
  printf '%s\n' '<memory_candidate><scope>project</scope><type>decision</type><topic_key>ticket-key-convention</topic_key><risk_class>low</risk_class><confidence>0.95</confidence><text>Ticket keys normalize to uppercase PREFIX-digits.</text></memory_candidate>' > "$output_path"
elif grep -q "Task: graph_candidate" "$input_path"; then
  printf '%s\n' '<no_graph_candidates reason="test stub has no graph facts"/>' > "$output_path"
else
  printf '%s\n' '{"observations":[{"type":"decision","title":"Ticket key memory convention","subtitle":null,"narrative":"Ticket keys normalize to uppercase PREFIX-digits. Inputs may contain spaces, underscores, hyphens, or a leading #. If the input contains only digits, use default prefix MEM.","facts":[],"concepts":[],"files_read":[],"files_modified":["memory_demo/tickets.py"],"confidence":0.95}]}' > "$output_path"
fi
"#,
        )?;
        let mut permissions = fs::metadata(&stub)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&stub, permissions)?;

        let config = root.join("config.toml");
        fs::write(
            &config,
            format!(
                r#"version = 1
[memory_ai]
default_host = "codex-cli"
[memory_ai.hosts."codex-cli"]
memory_profile = "codex"
context_gate = "strict"
context_color = false
capture_adapter = "codex-cli"
[memory_ai.profiles.codex]
executor = "codex-cli"
model = "test"
path = "{}"
"#,
                stub.display()
            ),
        )?;
        let fixture = super::super::fixture::load_fixture("eval/coding-bench/fixtures/tasks.json")?;
        let prepared = prepare_remem_e2e(&root.join("data"), &fixture.tasks[0], &config).await;
        let cleanup = fs::remove_dir_all(&root);
        let prepared = prepared?;
        cleanup?;

        assert_eq!(prepared.trace.captured_count, 1);
        assert!(prepared.trace.extracted_observation_count >= 1);
        assert!(prepared.trace.candidate_count >= 1);
        assert!(prepared.trace.promoted_memory_count >= 1);
        assert!(prepared.trace.retrieved_memory_count >= 1);
        assert!(!prepared.trace.pipeline_starved);
        assert!(prepared.rendered_context.contains("Ticket keys normalize"));
        Ok(())
    }
}
