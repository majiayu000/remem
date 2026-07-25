use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use crate::{
    db,
    memory::{self, suppression::SuppressionRecord, Memory},
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MemoryCurrentness {
    pub(super) expires_at_epoch: Option<i64>,
    pub(super) valid_from_epoch: Option<i64>,
    pub(super) valid_to_epoch: Option<i64>,
    pub(super) now_epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PackAttribution {
    pub(super) origin: String,
    pub(super) source_project: Option<String>,
    pub(super) routing_reason: Option<String>,
}

pub(in crate::cli) fn run_why(id: i64, project: Option<&str>, branch: Option<&str>) -> Result<()> {
    let conn = db::open_db()?;
    let memories = memory::get_memories_by_ids_with_suppressed_policy(&conn, &[id], None, true)?;

    let Some(memory) = memories.first() else {
        println!("Memory #{id} not found.");
        return Ok(());
    };

    let current_project = resolve_current_project(project)?;
    let current_branch = resolve_current_branch(branch)?;
    let gate_project = context_gate_project(memory, current_project.as_deref());
    let gate = load_latest_context_gate_summary(&conn, gate_project)?;
    let currentness = load_memory_currentness(&conn, memory.id)?;
    let pack_attribution = load_pack_attribution(&conn, memory.id)?;
    let suppressions = memory::suppression::active_suppressions_for_memory(&conn, memory.id)?;

    print!(
        "{}",
        render_why_memory(
            memory,
            current_project.as_deref(),
            current_branch.as_deref(),
            gate.as_ref(),
            currentness.as_ref(),
            pack_attribution.as_ref(),
            &suppressions,
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

fn load_memory_currentness(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<Option<MemoryCurrentness>> {
    let now_epoch = chrono::Utc::now().timestamp();
    conn.query_row(
        "SELECT expires_at_epoch, valid_from_epoch, valid_to_epoch
         FROM memories
         WHERE id = ?1",
        [id],
        |row| {
            Ok(MemoryCurrentness {
                expires_at_epoch: row.get(0)?,
                valid_from_epoch: row.get(1)?,
                valid_to_epoch: row.get(2)?,
                now_epoch,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn load_pack_attribution(conn: &rusqlite::Connection, id: i64) -> Result<Option<PackAttribution>> {
    let row = conn
        .query_row(
            "SELECT source_trust_class, source_project, topic_domain, routing_reason
             FROM memories
             WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((trust_class, source_project, topic_domain, routing_reason)) = row else {
        return Ok(None);
    };
    if trust_class != "pack"
        && !topic_domain
            .as_deref()
            .is_some_and(|v| v.starts_with("pack:"))
    {
        return Ok(None);
    }
    Ok(Some(PackAttribution {
        origin: topic_domain.unwrap_or_else(|| "pack:<unknown>".to_string()),
        source_project,
        routing_reason,
    }))
}

pub(super) fn render_why_memory(
    memory: &Memory,
    current_project: Option<&str>,
    current_branch: Option<&str>,
    gate: Option<&ContextGateSummary>,
    currentness: Option<&MemoryCurrentness>,
    pack_attribution: Option<&PackAttribution>,
    suppressions: &[SuppressionRecord],
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
        status_visibility(&memory.status, currentness)
    ));
    output.push_str(&format!(
        "  currentness: {}\n",
        currentness_visibility(currentness)
    ));
    output.push_str(&format!(
        "  suppression: {}\n",
        suppression_visibility(suppressions)
    ));
    if let Some(pack_attribution) = pack_attribution {
        output.push_str(&format!(
            "  pack attribution: {}\n",
            pack_attribution_visibility(pack_attribution)
        ));
    }
    output.push_str(&format!(
        "  recency: updated {}\n",
        format_memory_timestamp(memory.updated_at_epoch)
    ));
    output.push_str(
        "  query scoring: query-specific; run `remem search \"<query>\" --explain --limit 3`.\n",
    );
    output.push_str(&format!(
        "  context visibility: {}\n",
        context_visibility(&memory.memory_type, &memory.status, currentness)
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

fn pack_attribution_visibility(pack: &PackAttribution) -> String {
    let imported_from = pack
        .routing_reason
        .as_deref()
        .and_then(|reason| reason.strip_prefix("pack import from "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(pack.source_project.as_deref())
        .unwrap_or("unknown");
    format!(
        "origin={} imported_from={imported_from} trust=pack",
        pack.origin
    )
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

fn status_visibility(status: &str, currentness: Option<&MemoryCurrentness>) -> String {
    if let Some(currentness) = currentness {
        if status == "active" {
            if let Some(expires_at) = currentness.expires_at_epoch {
                if expires_at <= currentness.now_epoch {
                    return format!(
                        "active but expired at {}; hidden by default current retrieval until cleanup marks it stale",
                        format_memory_timestamp(expires_at)
                    );
                }
                return format!(
                    "active; default current retrieval can include it until {}",
                    format_memory_timestamp(expires_at)
                );
            }
        }
    }

    match status {
        "active" => "active; default search can include it".to_string(),
        "stale" | "archived" => {
            format!("{status}; hidden by default search, retry with --include-stale")
        }
        other => format!("{other}; not returned by the default active/stale/archive search filter"),
    }
}

fn suppression_visibility(suppressions: &[SuppressionRecord]) -> String {
    if suppressions.is_empty() {
        return "not suppressed by policy".to_string();
    }
    suppressions
        .iter()
        .map(|record| {
            let target = record
                .target_id
                .map(|id| format!("{}:{id}", record.target_kind))
                .or_else(|| {
                    record
                        .target_value
                        .as_ref()
                        .map(|value| format!("{}:{value}", record.target_kind))
                })
                .unwrap_or_else(|| record.target_kind.clone());
            format!(
                "suppressed by policy #{} target={} reason={} actor={}",
                record.id, target, record.reason, record.actor
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn currentness_visibility(currentness: Option<&MemoryCurrentness>) -> String {
    let Some(currentness) = currentness else {
        return "no TTL/currentness metadata available".to_string();
    };

    let expires = currentness
        .expires_at_epoch
        .map(format_memory_timestamp)
        .unwrap_or_else(|| "none".to_string());
    let valid_from = currentness
        .valid_from_epoch
        .map(format_memory_timestamp)
        .unwrap_or_else(|| "none".to_string());
    let valid_to = currentness
        .valid_to_epoch
        .map(format_memory_timestamp)
        .unwrap_or_else(|| "none".to_string());
    format!("expires={expires}; valid_from={valid_from}; valid_to={valid_to}")
}

fn context_visibility(
    memory_type: &str,
    status: &str,
    currentness: Option<&MemoryCurrentness>,
) -> String {
    if status != "active" {
        return format!("not context-eligible by default because status={status}");
    }
    if let Some(currentness) = currentness {
        if currentness
            .expires_at_epoch
            .is_some_and(|expires_at| expires_at <= currentness.now_epoch)
        {
            return "not context-eligible by default because the current fact is expired"
                .to_string();
        }
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
