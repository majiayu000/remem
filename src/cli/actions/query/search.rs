use anyhow::Result;
use serde::Serialize;

use crate::{
    db,
    memory::{
        raw_archive::RawMessage,
        service::{MultiHopMeta, SearchRequest, SearchResultSet},
        Memory,
    },
    retrieval::search::SearchExplain,
};

pub(in crate::cli) fn run_search(
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    branch: Option<&str>,
    include_stale: bool,
    multi_hop: bool,
    explain: bool,
    json: bool,
) -> Result<()> {
    let conn = db::open_db()?;
    let request = build_search_request(
        query,
        project,
        memory_type,
        limit,
        offset,
        branch,
        include_stale,
        multi_hop,
        explain,
    );
    let results = crate::memory::service::search_memories(&conn, &request)?;
    if json {
        let output = build_search_json(
            query,
            project,
            memory_type,
            limit,
            offset,
            branch,
            include_stale,
            multi_hop,
            explain,
            &results,
        );
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    print!("{}", render_search_results(&results, offset, limit.max(1)));
    Ok(())
}

pub(super) fn build_search_request(
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    branch: Option<&str>,
    include_stale: bool,
    multi_hop: bool,
    explain: bool,
) -> SearchRequest {
    SearchRequest {
        query: Some(query.to_string()),
        project: project.map(str::to_string),
        memory_type: memory_type.map(str::to_string),
        limit,
        offset,
        include_stale,
        branch: branch.map(str::to_string),
        multi_hop,
        explain,
    }
}

pub(super) fn render_search_results(results: &SearchResultSet, offset: i64, limit: i64) -> String {
    let mut output = String::new();
    if results.memories.is_empty() && results.raw_hits.is_empty() {
        output.push_str("No curated memories found.\n");
        append_empty_search_guidance(&mut output);
        append_search_explain(&mut output, results.explain.as_ref());
        return output;
    }

    if let Some(meta) = results.multi_hop.as_ref() {
        render_multi_hop_meta(&mut output, meta);
    }

    if results.memories.is_empty() {
        output.push_str("No curated memories found.\n");
    } else {
        output.push_str(&format!("Found {} result(s):\n\n", results.memories.len()));
        for memory in &results.memories {
            output.push_str(&format_memory_line(memory));
        }
        append_curated_next_steps(
            &mut output,
            &results.memories[0],
            results.has_more,
            offset,
            limit,
        );
    }

    if !results.raw_hits.is_empty() {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push('\n');
        output.push_str("Raw archive fallback:\n");
        for raw in &results.raw_hits {
            output.push_str(&format_raw_hit_line(raw));
        }
        append_raw_fallback_next_step(&mut output);
    }
    append_search_explain(&mut output, results.explain.as_ref());

    output
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_search_json(
    query: &str,
    project: Option<&str>,
    memory_type: Option<&str>,
    limit: i64,
    offset: i64,
    branch: Option<&str>,
    include_stale: bool,
    multi_hop: bool,
    explain: bool,
    results: &SearchResultSet,
) -> SearchJson {
    let normalized_limit = limit.max(1);
    SearchJson {
        query: query.to_string(),
        project: project.map(str::to_string),
        memory_type: memory_type.map(str::to_string),
        limit: normalized_limit,
        offset: offset.max(0),
        branch: branch.map(str::to_string),
        include_stale,
        multi_hop_requested: multi_hop,
        explain_requested: explain,
        count: results.memories.len(),
        has_more: results.has_more,
        next_offset: results.has_more.then_some(offset.max(0) + normalized_limit),
        results: results.memories.clone(),
        raw_hits: results
            .raw_hits
            .iter()
            .map(|raw| RawHitJson {
                id: raw.id,
                session_id: raw.session_id.clone(),
                project: raw.project.clone(),
                role: raw.role.clone(),
                content: raw.content.clone(),
                source: raw.source.clone(),
                branch: raw.branch.clone(),
                cwd: raw.cwd.clone(),
                created_at_epoch: raw.created_at_epoch,
            })
            .collect(),
        multi_hop: results.multi_hop.as_ref().map(|meta| MultiHopJson {
            hops: meta.hops,
            entities_discovered: meta.entities_discovered.clone(),
        }),
        explain_details: results.explain.clone(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SearchJson {
    pub query: String,
    pub project: Option<String>,
    pub memory_type: Option<String>,
    pub limit: i64,
    pub offset: i64,
    pub branch: Option<String>,
    pub include_stale: bool,
    pub multi_hop_requested: bool,
    pub explain_requested: bool,
    pub count: usize,
    pub has_more: bool,
    pub next_offset: Option<i64>,
    pub results: Vec<Memory>,
    pub raw_hits: Vec<RawHitJson>,
    pub multi_hop: Option<MultiHopJson>,
    pub explain_details: Option<SearchExplain>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct MultiHopJson {
    pub hops: u8,
    pub entities_discovered: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct RawHitJson {
    pub id: i64,
    pub session_id: String,
    pub project: String,
    pub role: String,
    pub content: String,
    pub source: String,
    pub branch: Option<String>,
    pub cwd: Option<String>,
    pub created_at_epoch: i64,
}

fn append_empty_search_guidance(output: &mut String) {
    output.push_str("\nTry:\n");
    output.push_str("  remem search \"<query>\" --include-stale\n");
    output.push_str("  remem search \"<query>\" --multi-hop\n");
    output.push_str("  remem search \"<query>\" --project /path/to/repo\n");
    output.push_str("  remem search \"<query>\" --explain\n");
}

fn append_curated_next_steps(
    output: &mut String,
    first_memory: &Memory,
    has_more: bool,
    offset: i64,
    limit: i64,
) {
    output.push_str("\nNext:\n");
    output.push_str(&format!(
        "  remem show {}        # full details for one memory\n",
        first_memory.id
    ));
    output.push_str(&format!(
        "  remem why {}         # visibility and retrieval diagnostics\n",
        first_memory.id
    ));
    if has_more {
        output.push_str(&format!(
            "  remem search \"<query>\" --offset {}\n",
            offset.max(0) + limit.max(1)
        ));
    }
}

fn append_raw_fallback_next_step(output: &mut String) {
    output.push_str("\nNext:\n");
    output.push_str(
        "  use raw hits for recall only; promote durable conclusions with review/save_memory.\n",
    );
}

fn append_search_explain(output: &mut String, explain: Option<&SearchExplain>) {
    if let Some(explain) = explain {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push('\n');
        output.push_str(&render_search_explain(explain));
    }
}

fn render_multi_hop_meta(output: &mut String, meta: &MultiHopMeta) {
    output.push_str(&format!("Multi-hop: hops={}", meta.hops));
    if !meta.entities_discovered.is_empty() {
        output.push_str(&format!(
            " entities={}",
            meta.entities_discovered.join(", ")
        ));
    }
    output.push_str("\n\n");
}

fn format_memory_line(memory: &Memory) -> String {
    let mut output = format!(
        "  [{}] {} | {} | {} | {}\n",
        memory.id,
        memory.memory_type,
        memory.project,
        created_date(memory.created_at_epoch),
        memory.title
    );
    let preview = preview_text(memory);
    if !preview.is_empty() && preview != memory.title {
        output.push_str(&format!("       {}\n", preview));
    }
    output
}

fn format_raw_hit_line(raw: &RawMessage) -> String {
    let branch = raw
        .branch
        .as_deref()
        .map(|branch| format!(" | branch={branch}"))
        .unwrap_or_default();
    let preview = preview_raw_text(raw);
    let mut output = format!(
        "  [raw:{}] {} | {} | {}{}",
        raw.id,
        raw.role,
        raw.project,
        created_date(raw.created_at_epoch),
        branch
    );
    if !preview.is_empty() {
        output.push_str(&format!(" | {}", preview));
    }
    output.push('\n');
    output
}

fn render_search_explain(explain: &SearchExplain) -> String {
    let mut output = String::new();
    output.push_str("Search explain:\n");
    output.push_str(&format!("  query: {:?}\n", explain.query));
    output.push_str(&format!(
        "  filters: project={:?} branch={:?} type={:?} include_stale={}\n",
        explain.project, explain.branch, explain.memory_type, explain.include_stale
    ));
    output.push_str(&format!(
        "  pagination: limit={} offset={} fetch_limit={} has_more={}\n",
        explain.limit, explain.offset, explain.fetch_limit, explain.has_more
    ));
    output.push_str(&format!(
        "  expanded_terms: [{}]\n",
        explain.expanded_terms.join(", ")
    ));
    output.push_str(&format!(
        "  core_terms: [{}]\n",
        explain.core_terms.join(", ")
    ));
    output.push_str(&format!("  fts_query: {:?}\n", explain.fts_query));
    output.push_str(&format!("  temporal_range: {:?}\n", explain.temporal_range));
    output.push_str(&format!("  temporal_field: {:?}\n", explain.temporal_field));
    output.push_str(&format!("  rrf_k: {:.1}\n", explain.rrf_k));
    output.push_str("  channels:\n");
    for channel in &explain.channels {
        output.push_str(&format!(
            "    {}: {}\n",
            channel.name,
            channel
                .hits
                .iter()
                .map(|hit| format!("{}#{}", hit.memory_id, hit.rank))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    output.push_str("  results:\n");
    for result in &explain.results {
        output.push_str(&format!(
            "    [{}] rank={} score={:.6} visibility={} scope={} project={}\n",
            result.memory_id,
            result.final_rank,
            result.final_score,
            result.visibility,
            result.scope,
            result.project
        ));
        if !result.contributions.is_empty() {
            output.push_str(&format!(
                "      contributions: {}\n",
                result
                    .contributions
                    .iter()
                    .map(|contribution| format!(
                        "{}#{}={:.6}",
                        contribution.channel, contribution.rank, contribution.score
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    output.push_str(&format!(
        "  raw_fallback_count: {}\n",
        explain.raw_fallback_count
    ));
    output
}

pub(super) fn created_date(created_at_epoch: i64) -> String {
    chrono::DateTime::from_timestamp(created_at_epoch, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

pub(super) fn preview_text(memory: &Memory) -> String {
    memory
        .text
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(80)
        .collect()
}

pub(super) fn preview_raw_text(raw: &RawMessage) -> String {
    raw.content
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(100)
        .collect()
}
