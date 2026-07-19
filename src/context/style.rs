use super::host::HostKind;
use super::render::ContextRenderStats;

const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BOLD_CYAN: &str = "\x1b[1;36m";
const ANSI_BOLD: &str = "\x1b[1m";

pub(in crate::context) fn context_header(
    project: &str,
    current_branch: Option<&str>,
    hook_source: Option<&str>,
    host: HostKind,
    use_colors: bool,
) -> String {
    let mut rows = vec![("project", project.to_string())];
    if let Some(branch) = current_branch {
        rows.push(("branch", branch.to_string()));
    }
    let source = context_source_footer(hook_source);
    if source != "-" {
        rows.push(("source", source.to_string()));
    }

    let mut header = String::new();
    let row_indent = if host == HostKind::CodexCli && use_colors {
        "              "
    } else {
        ""
    };
    push_rail(&mut header, "remem context", &rows, row_indent, use_colors);
    header.push('\n');
    header
}

pub(in crate::context) fn context_delta_title_line_like(
    _first_line: &str,
    use_colors: bool,
) -> String {
    let mut header = String::new();
    header.push_str(&rail_title_line("remem context delta", use_colors));
    header
}

pub(in crate::context) fn context_stats_footer(
    stats: &ContextRenderStats,
    use_colors: bool,
) -> String {
    let estimated_tokens = estimate_display_tokens(stats.output_chars);
    let rows = vec![
        (
            "Memories",
            format!(
                "{} total, {} core, {} lessons, {} indexed",
                stats.memories_loaded, stats.core.count, stats.lessons.count, stats.index.count
            ),
        ),
        (
            "Preferences",
            format!(
                "{} total, {} project, {} global",
                stats.preferences.count, stats.project_preferences, stats.global_preferences
            ),
        ),
        ("Sessions", stats.sessions.count.to_string()),
        ("Workstreams", stats.workstreams.count.to_string()),
        ("Relevance", {
            let threshold = stats
                .relevance
                .threshold
                .map(|value| format!("{value:.3}"))
                .unwrap_or_else(|| "-".to_string());
            format!(
                    "{} (k={}, threshold={}, candidates={}, eligible={}, injected={}, low={}, k_dropped={})",
                    stats.relevance.state,
                    stats.relevance.k,
                    threshold,
                    stats.relevance.candidates,
                    stats.relevance.eligible,
                    stats.relevance.final_injected,
                    stats.relevance.below_threshold,
                    stats.relevance.k_limited
                )
        }),
        (
            "Budget",
            format!(
                "{} chars (~{} tokens) / {}, truncated: {}",
                stats.output_chars,
                estimated_tokens,
                stats.total_char_limit,
                if stats.truncated { "yes" } else { "no" }
            ),
        ),
    ];
    let mut footer = String::new();
    footer.push('\n');
    push_rail(&mut footer, "Loaded", &rows, "", use_colors);
    footer
}

pub(in crate::context) fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            output.push(ch);
            continue;
        }

        if chars.next_if_eq(&'[').is_some() {
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
        }
    }
    output
}

pub(in crate::context) fn contains_ansi(input: &str) -> bool {
    input.contains("\x1b[")
}

fn push_rail(
    output: &mut String,
    title: &str,
    rows: &[(&str, String)],
    row_indent: &str,
    use_colors: bool,
) {
    output.push_str(&rail_title_line(title, use_colors));
    for (idx, (label, value)) in rows.iter().enumerate() {
        let marker = if idx + 1 == rows.len() {
            "└─"
        } else {
            "├─"
        };
        output.push_str(&rail_row_line(row_indent, marker, label, value, use_colors));
    }
}

fn rail_title_line(title: &str, use_colors: bool) -> String {
    let title = if use_colors {
        paint(title, ANSI_BOLD_CYAN)
    } else {
        title.to_string()
    };
    format!("{title}\n")
}

fn rail_row_line(
    row_indent: &str,
    marker: &str,
    label: &str,
    value: &str,
    use_colors: bool,
) -> String {
    let label = if use_colors {
        paint(label, ANSI_BOLD)
    } else {
        label.to_string()
    };
    format!("{row_indent}{marker} {label}: {value}\n")
}

fn context_source_footer(source: Option<&str>) -> &'static str {
    match source
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("compact") => "compact",
        Some("clear") => "clear",
        _ => "-",
    }
}

fn estimate_display_tokens(chars: usize) -> usize {
    (chars + 3) / 4
}

fn paint(value: &str, ansi_code: &str) -> String {
    format!("{ansi_code}{value}{ANSI_RESET}")
}
