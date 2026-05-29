use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use crate::{
    db,
    memory::{self, Memory},
};

use super::show::format_memory_timestamp;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContextGateSummary {
    pub(super) host: String,
    pub(super) project: String,
    pub(super) output_mode: String,
    pub(super) emit_count: i64,
    pub(super) suppress_count: i64,
    pub(super) updated_at_epoch: i64,
    pub(super) last_emitted_epoch: i64,
}

pub(in crate::cli) fn run_why(id: i64, project: Option<&str>, branch: Option<&str>) -> Result<()> {
    let conn = db::open_db()?;
    let memories = memory::get_memories_by_ids(&conn, &[id], None)?;

    let Some(memory) = memories.first() else {
        println!("Memory #{id} not found.");
        return Ok(());
    };

    let current_project = resolve_current_project(project)?;
    let current_branch = resolve_current_branch(branch)?;
    let gate_project = context_gate_project(memory, current_project.as_deref());
    let gate = load_latest_context_gate_summary(&conn, gate_project)?;

    print!(
        "{}",
        render_why_memory(
            memory,
            current_project.as_deref(),
            current_branch.as_deref(),
            gate.as_ref()
        )
    );
    Ok(())
}

fn resolve_current_project(explicit: Option<&str>) -> Result<Option<String>> {
    if let Some(project) = explicit {
        return Ok(Some(project.to_string()));
    }
    let cwd = std::env::current_dir().context("resolve current directory for why diagnostics")?;
    Ok(Some(db::project_from_cwd(cwd.to_string_lossy().as_ref())))
}

fn resolve_current_branch(explicit: Option<&str>) -> Result<Option<String>> {
    if let Some(branch) = explicit {
        return Ok(Some(branch.to_string()));
    }
    let cwd =
        std::env::current_dir().context("resolve current directory for branch diagnostics")?;
    Ok(db::detect_git_branch(cwd.to_string_lossy().as_ref()))
}

fn context_gate_project<'a>(memory: &'a Memory, current_project: Option<&'a str>) -> &'a str {
    if memory.scope == "global" {
        current_project.unwrap_or(&memory.project)
    } else {
        &memory.project
    }
}

fn load_latest_context_gate_summary(
    conn: &rusqlite::Connection,
    project: &str,
) -> Result<Option<ContextGateSummary>> {
    conn.query_row(
        "SELECT host, project, output_mode, emit_count, suppress_count, updated_at_epoch,
                last_emitted_epoch
         FROM context_injections
         WHERE project = ?1
         ORDER BY updated_at_epoch DESC
         LIMIT 1",
        [project],
        |row| {
            Ok(ContextGateSummary {
                host: row.get(0)?,
                project: row.get(1)?,
                output_mode: row.get(2)?,
                emit_count: row.get(3)?,
                suppress_count: row.get(4)?,
                updated_at_epoch: row.get(5)?,
                last_emitted_epoch: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn render_why_memory(
    memory: &Memory,
    current_project: Option<&str>,
    current_branch: Option<&str>,
    gate: Option<&ContextGateSummary>,
) -> String {
    let mut output = String::new();
    output.push_str(&format!("Memory #{}\n", memory.id));
    output.push_str(&format!("  title: {}\n", memory.title));
    output.push_str(&format!("  scope: {}\n", memory.scope));
    output.push_str(&format!(
        "  project match: {}\n",
        project_visibility(memory, current_project)
    ));
    output.push_str(&format!(
        "  branch match: {}\n",
        branch_visibility(memory.branch.as_deref(), current_branch)
    ));
    output.push_str(&format!(
        "  type: {}\n",
        type_visibility(&memory.memory_type)
    ));
    output.push_str(&format!(
        "  status: {}\n",
        status_visibility(&memory.status)
    ));
    output.push_str(&format!(
        "  recency: updated {}\n",
        format_memory_timestamp(memory.updated_at_epoch)
    ));
    output.push_str(
        "  query scoring: query-specific; run `remem search \"<query>\" --explain --limit 3`.\n",
    );
    output.push_str(&format!(
        "  context visibility: {}\n",
        context_visibility(&memory.memory_type, &memory.status)
    ));
    output.push_str(&format!(
        "  context gate: {}\n",
        context_gate_visibility(gate)
    ));
    output.push_str(&format!(
        "\nNext:\n  remem show {}\n  remem search \"<query>\" --explain --limit 3\n",
        memory.id
    ));
    output
}

fn project_visibility(memory: &Memory, current_project: Option<&str>) -> String {
    if memory.scope == "global" {
        return current_project
            .map(|project| format!("global memory visible from {project}"))
            .unwrap_or_else(|| "global memory visible from any project".to_string());
    }

    match current_project {
        Some(project) if project == memory.project => format!("exact {project}"),
        Some(project) => format!(
            "mismatch: memory project={} current project={project}",
            memory.project
        ),
        None => format!("not evaluated; memory project={}", memory.project),
    }
}

fn branch_visibility(memory_branch: Option<&str>, current_branch: Option<&str>) -> String {
    match (memory_branch, current_branch) {
        (Some(memory_branch), Some(current_branch)) if memory_branch == current_branch => {
            format!("exact {current_branch}")
        }
        (Some(memory_branch), Some(current_branch)) => format!(
            "mismatch: memory branch={memory_branch} current branch={current_branch}; branch-scoped search filters it out"
        ),
        (None, Some(current_branch)) => {
            format!("branchless; visible in branch-scoped search for {current_branch}")
        }
        (Some(memory_branch), None) => {
            format!("memory branch={memory_branch}; unfiltered search can include it")
        }
        (None, None) => "branchless; visible without a branch filter".to_string(),
    }
}

fn type_visibility(memory_type: &str) -> String {
    match memory_type {
        "preference" => {
            "preference; search-visible and rendered in the preferences context section"
        }
        "lesson" => "lesson; search-visible and rendered in the lessons context section",
        "bugfix" | "architecture" | "decision" | "discovery" => {
            "core memory type; search-visible and eligible for core context"
        }
        "session_activity" => "session_activity; search-visible but excluded from core context",
        "procedure" => "procedure; search-visible and eligible for the memory index",
        _ => "custom type; search-visible when status/project/branch filters match",
    }
    .to_string()
}

fn status_visibility(status: &str) -> String {
    match status {
        "active" => "active; default search can include it".to_string(),
        "stale" | "archived" => {
            format!("{status}; hidden by default search, retry with --include-stale")
        }
        other => format!("{other}; not returned by the default active/stale/archive search filter"),
    }
}

fn context_visibility(memory_type: &str, status: &str) -> String {
    if status != "active" {
        return format!("not context-eligible by default because status={status}");
    }

    match memory_type {
        "preference" => {
            "preferences section candidate; excluded from memory index/core".to_string()
        }
        "lesson" => "lessons section candidate; excluded from memory index/core".to_string(),
        "bugfix" | "architecture" | "decision" | "discovery" => {
            "memory index candidate and core candidate".to_string()
        }
        "session_activity" => "memory index candidate; excluded from core".to_string(),
        _ => "memory index candidate if project/branch filters match".to_string(),
    }
}

fn context_gate_visibility(gate: Option<&ContextGateSummary>) -> String {
    let Some(gate) = gate else {
        return "no recent context gate row for this project".to_string();
    };

    format!(
        "latest {host} output for {project}: mode={mode}, emitted={emitted}, suppressed={suppressed}, updated={updated}, last_emitted={last_emitted}; gate rows are context-output level, not per-memory proof",
        host = gate.host,
        project = gate.project,
        mode = gate.output_mode,
        emitted = gate.emit_count,
        suppressed = gate.suppress_count,
        updated = format_memory_timestamp(gate.updated_at_epoch),
        last_emitted = format_memory_timestamp(gate.last_emitted_epoch),
    )
}
