use anyhow::Result;

use crate::{
    db,
    memory::{
        raw_archive::RawMessage,
        service::{MultiHopMeta, SearchRequest, SearchResultSet},
        Memory,
    },
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
    );
    let results = crate::memory::service::search_memories(&conn, &request)?;
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
    }
}

pub(super) fn render_search_results(results: &SearchResultSet, offset: i64, limit: i64) -> String {
    let mut output = String::new();
    if results.memories.is_empty() && results.raw_hits.is_empty() {
        output.push_str("No results found.\n");
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
        if results.has_more {
            output.push_str(&format!(
                "\nMore results available; use --offset {}.\n",
                offset.max(0) + limit.max(1)
            ));
        }
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
    }

    output
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
