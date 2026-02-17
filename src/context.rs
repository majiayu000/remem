use anyhow::Result;
use chrono::{Local, TimeZone};
use std::collections::HashSet;

use crate::db::{self, Observation, SessionSummary};

const CHARS_PER_TOKEN: usize = 4;
const SUMMARY_LOOKAHEAD: i64 = 1;

struct ContextConfig {
    total_observation_count: i64,
    full_observation_count: i64,
    session_count: i64,
    show_read_tokens: bool,
    show_work_tokens: bool,
    observation_types: Vec<String>,
    show_last_summary: bool,
    full_observation_field: String,
}

fn load_config() -> ContextConfig {
    let get = |key: &str, default: &str| -> String {
        std::env::var(key).unwrap_or_else(|_| default.to_string())
    };

    let types_str = get(
        "CLAUDE_MEM_CONTEXT_OBSERVATION_TYPES",
        "bugfix,feature,refactor,discovery,decision,change",
    );
    let observation_types: Vec<String> = types_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    ContextConfig {
        total_observation_count: get("CLAUDE_MEM_CONTEXT_OBSERVATIONS", "50")
            .parse()
            .unwrap_or(50),
        full_observation_count: get("CLAUDE_MEM_CONTEXT_FULL_COUNT", "5")
            .parse()
            .unwrap_or(5),
        session_count: get("CLAUDE_MEM_CONTEXT_SESSION_COUNT", "10")
            .parse()
            .unwrap_or(10),
        show_read_tokens: get("CLAUDE_MEM_CONTEXT_SHOW_READ_TOKENS", "true") == "true",
        show_work_tokens: get("CLAUDE_MEM_CONTEXT_SHOW_WORK_TOKENS", "true") == "true",
        observation_types,
        show_last_summary: get("CLAUDE_MEM_CONTEXT_SHOW_LAST_SUMMARY", "true") == "true",
        full_observation_field: get("CLAUDE_MEM_CONTEXT_FULL_FIELD", "narrative"),
    }
}

fn project_from_cwd(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| cwd.to_string())
}

fn format_header_datetime() -> String {
    Local::now().format("%Y-%m-%d %-I:%M%P %Z").to_string()
}

fn type_emoji(t: &str) -> &'static str {
    match t {
        "decision" => "\u{1f535}",
        "bugfix" => "\u{1f41b}",
        "feature" => "\u{2728}",
        "refactor" => "\u{1f527}",
        "discovery" => "\u{1f50d}",
        "change" => "\u{1f504}",
        _ => "\u{25cf}",
    }
}

fn calc_observation_tokens(obs: &Observation) -> usize {
    let size = obs.title.as_deref().map_or(0, |s| s.len())
        + obs.subtitle.as_deref().map_or(0, |s| s.len())
        + obs.narrative.as_deref().map_or(0, |s| s.len())
        + obs.facts.as_deref().map_or(0, |s| s.len());
    (size + CHARS_PER_TOKEN - 1) / CHARS_PER_TOKEN
}

struct TokenEconomics {
    total_observations: usize,
    total_read_tokens: usize,
    total_discovery_tokens: i64,
    savings: i64,
    savings_percent: i64,
}

fn calculate_token_economics(observations: &[Observation]) -> TokenEconomics {
    let total_observations = observations.len();
    let total_read_tokens: usize = observations.iter().map(calc_observation_tokens).sum();
    let total_discovery_tokens: i64 = observations
        .iter()
        .map(|o| o.discovery_tokens.unwrap_or(0))
        .sum();
    let savings = total_discovery_tokens - total_read_tokens as i64;
    let savings_percent = if total_discovery_tokens > 0 {
        (savings * 100) / total_discovery_tokens
    } else {
        0
    };
    TokenEconomics {
        total_observations,
        total_read_tokens,
        total_discovery_tokens,
        savings,
        savings_percent,
    }
}

enum TimelineItem {
    Obs(Observation),
    Sum(SummaryTimelineItem),
}

struct SummaryTimelineItem {
    summary: SessionSummary,
    display_epoch: i64,
}

fn epoch_to_secs(epoch: i64) -> i64 {
    if epoch > 9_999_999_999 { epoch / 1000 } else { epoch }
}

fn format_epoch_time(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch_to_secs(epoch), 0)
        .single()
        .map(|dt| dt.format("%-I:%M %p").to_string())
        .unwrap_or_default()
}

fn format_epoch_date(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch_to_secs(epoch), 0)
        .single()
        .map(|dt| dt.format("%b %d, %Y").to_string())
        .unwrap_or_default()
}

pub fn generate_context(cwd: &str, _session_id: Option<&str>, use_colors: bool) -> Result<()> {
    let timer = crate::log::Timer::start("context", &format!("cwd={}", cwd));
    let config = load_config();
    let project = project_from_cwd(cwd);

    let conn = match db::open_db_readonly() {
        Ok(c) => c,
        Err(_) => {
            crate::log::info("context", &format!("no DB, empty state for project={}", project));
            render_empty_state(&project, use_colors);
            timer.done("empty (no DB)");
            return Ok(());
        }
    };

    let type_refs: Vec<&str> = config.observation_types.iter().map(|s| s.as_str()).collect();
    let observations = db::query_observations(
        &conn,
        &project,
        &type_refs,
        config.total_observation_count,
    )?;

    let summaries = db::query_summaries(
        &conn,
        &project,
        config.session_count + SUMMARY_LOOKAHEAD,
    )?;

    if observations.is_empty() && summaries.is_empty() {
        crate::log::info("context", &format!("no data for project={}", project));
        render_empty_state(&project, use_colors);
        timer.done("empty (no data)");
        return Ok(());
    }

    let economics = calculate_token_economics(&observations);

    let mut output = String::new();

    // Header
    output.push_str(&format!(
        "# [{}] recent context, {}\n\n",
        project,
        format_header_datetime()
    ));

    // Legend
    let type_legend: Vec<String> = config
        .observation_types
        .iter()
        .map(|t| format!("{} {}", type_emoji(t), t))
        .collect();
    output.push_str(&format!(
        "**Legend:** session-request | {}\n\n",
        type_legend.join(" | ")
    ));

    // Context Index
    output.push_str(
        "**Context Index:** This compact index is usually sufficient. Only fetch details when needed.\n\n\
         **On-demand retrieval** (MCP remem tools):\n\
         - `search(query)` → find relevant observation IDs by keyword\n\
         - `get_observations(ids)` → fetch full narrative, facts, concepts, files\n\
         - `timeline(query)` → chronological context around a change\n\n\
         **When to fetch:** user asks about past work, you need implementation details, \
         debugging a previously-fixed issue, or looking for architecture decisions.\n\
         **Trust this index** over re-reading code for past decisions and learnings.\n\n",
    );

    // Economics
    if config.show_read_tokens || config.show_work_tokens {
        output.push_str(&format!(
            "**Context Economics**:\n\
             - Loading: {} observations ({} tokens to read)\n\
             - Work investment: {} tokens spent on research, building, and decisions\n\
             - Your savings: {} tokens ({}% reduction from reuse)\n\n",
            economics.total_observations,
            economics.total_read_tokens,
            economics.total_discovery_tokens,
            economics.savings,
            economics.savings_percent,
        ));
    }

    // Build timeline
    let full_ids: HashSet<i64> = observations
        .iter()
        .take(config.full_observation_count as usize)
        .map(|o| o.id)
        .collect();

    // Display summaries (skip most recent for timeline, show it separately)
    let display_summaries: Vec<&SessionSummary> = if summaries.len() > 1 {
        summaries[1..].iter().take(config.session_count as usize).collect()
    } else {
        vec![]
    };

    let mut timeline: Vec<TimelineItem> = Vec::new();
    for obs in &observations {
        timeline.push(TimelineItem::Obs(obs.clone()));
    }
    for (i, summary) in display_summaries.iter().enumerate() {
        let display_epoch = if i + 2 < summaries.len() {
            summaries[i + 2].created_at_epoch
        } else {
            summary.created_at_epoch
        };
        timeline.push(TimelineItem::Sum(SummaryTimelineItem {
            summary: (*summary).clone(),
            display_epoch,
        }));
    }
    timeline.sort_by_key(|item| match item {
        TimelineItem::Obs(o) => o.created_at_epoch,
        TimelineItem::Sum(s) => s.display_epoch,
    });

    // Render timeline grouped by day and session
    render_timeline(&mut output, &timeline, &full_ids, &config, cwd);

    // Most recent summary
    if config.show_last_summary {
        if let Some(latest) = summaries.first() {
            output.push_str("\n---\n\n");
            render_summary_fields(&mut output, latest);
        }
    }

    // Footer
    let work_tokens_k = (economics.total_discovery_tokens as f64 / 1000.0).round() as i64;
    output.push_str(&format!(
        "\nAccess {}k tokens of past research & decisions for just {}t. Use MCP search tools to access memories by ID.\n",
        work_tokens_k, economics.total_read_tokens
    ));

    print!("{}", output);

    timer.done(&format!(
        "project={} obs={} summaries={} read_tokens={} savings={}%",
        project,
        economics.total_observations,
        summaries.len(),
        economics.total_read_tokens,
        economics.savings_percent,
    ));
    Ok(())
}

fn render_timeline(
    output: &mut String,
    timeline: &[TimelineItem],
    full_ids: &HashSet<i64>,
    config: &ContextConfig,
    _cwd: &str,
) {
    let mut current_day = String::new();
    let mut current_session = String::new();
    let mut last_time = String::new();

    for item in timeline {
        match item {
            TimelineItem::Sum(s) => {
                let day = format_epoch_date(s.display_epoch);
                if day != current_day {
                    output.push_str(&format!("\n### {}\n\n", day));
                    current_day = day;
                    current_session.clear();
                }
                let request = s.summary.request.as_deref().unwrap_or("Session started");
                let time = format_epoch_time(s.display_epoch);
                output.push_str(&format!(
                    "**#S{}** {} ({})\n\n",
                    s.summary.id, request, time
                ));
            }
            TimelineItem::Obs(obs) => {
                let day = format_epoch_date(obs.created_at_epoch);
                if day != current_day {
                    output.push_str(&format!("\n### {}\n\n", day));
                    current_day = day;
                    current_session.clear();
                    last_time.clear();
                }

                let session = &obs.memory_session_id;
                if *session != current_session {
                    // New session — render table header
                    output.push_str(&format!("**{}**\n", session));
                    let mut header = "| ID | Time | T | Title |".to_string();
                    let mut sep = "|----|------|---|-------|".to_string();
                    if config.show_read_tokens {
                        header.push_str(" Read |");
                        sep.push_str("------|");
                    }
                    if config.show_work_tokens {
                        header.push_str(" Work |");
                        sep.push_str("------|");
                    }
                    output.push_str(&header);
                    output.push('\n');
                    output.push_str(&sep);
                    output.push('\n');
                    current_session = session.clone();
                    last_time.clear();
                }

                if full_ids.contains(&obs.id) {
                    render_full_observation(output, obs, config, &mut last_time);
                } else {
                    render_table_row(output, obs, config, &mut last_time);
                }
            }
        }
    }
}

fn render_table_row(
    output: &mut String,
    obs: &Observation,
    config: &ContextConfig,
    last_time: &mut String,
) {
    let time = format_epoch_time(obs.created_at_epoch);
    let time_display = if time == *last_time {
        "\"".to_string()
    } else {
        *last_time = time.clone();
        time
    };
    let icon = type_emoji(&obs.r#type);
    let title = obs.title.as_deref().unwrap_or("-");
    let read_tokens = calc_observation_tokens(obs);

    let mut row = format!("| #{} | {} | {} | {} |", obs.id, time_display, icon, title);
    if config.show_read_tokens {
        row.push_str(&format!(" ~{} |", read_tokens));
    }
    if config.show_work_tokens {
        let dt = obs.discovery_tokens.unwrap_or(0);
        let work = if dt > 0 {
            format!("{}", dt)
        } else {
            "-".to_string()
        };
        row.push_str(&format!(" {} |", work));
    }
    output.push_str(&row);
    output.push('\n');
}

fn render_full_observation(
    output: &mut String,
    obs: &Observation,
    config: &ContextConfig,
    last_time: &mut String,
) {
    let time = format_epoch_time(obs.created_at_epoch);
    let time_display = if time == *last_time {
        "\"".to_string()
    } else {
        *last_time = time.clone();
        time
    };
    let icon = type_emoji(&obs.r#type);
    let title = obs.title.as_deref().unwrap_or("-");
    let read_tokens = calc_observation_tokens(obs);
    let dt = obs.discovery_tokens.unwrap_or(0);

    output.push_str(&format!(
        "\n**#{}** {} {} **{}**\n\n",
        obs.id, time_display, icon, title
    ));

    let detail = if config.full_observation_field == "facts" {
        obs.facts.as_deref().unwrap_or("")
    } else {
        obs.narrative.as_deref().unwrap_or("")
    };
    if !detail.is_empty() {
        output.push_str(detail);
        output.push_str("\n\n");
    }

    let mut meta = format!("Read: ~{}", read_tokens);
    if dt > 0 {
        meta.push_str(&format!(", Work: {}", dt));
    }
    output.push_str(&meta);
    output.push_str("\n\n");
}

fn render_summary_fields(output: &mut String, summary: &SessionSummary) {
    let fields = [
        ("Request", &summary.request),
        ("Investigated", &summary.investigated),
        ("Learned", &summary.learned),
        ("Completed", &summary.completed),
        ("Next Steps", &summary.next_steps),
    ];
    for (label, value) in &fields {
        if let Some(v) = value {
            if !v.is_empty() {
                output.push_str(&format!("**{}**: {}\n\n", label, v));
            }
        }
    }
}

fn render_empty_state(project: &str, _use_colors: bool) {
    println!(
        "# [{}] recent context, {}\n\nNo previous sessions found for this project yet.",
        project,
        format_header_datetime()
    );
}
