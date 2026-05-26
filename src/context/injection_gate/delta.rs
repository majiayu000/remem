const DEFAULT_DELTA_CHAR_LIMIT: usize = 1200;

pub(super) fn build_delta_output(output: &str) -> String {
    let limit = read_usize_env("REMEM_CONTEXT_DELTA_CHAR_LIMIT", DEFAULT_DELTA_CHAR_LIMIT);
    if limit == 0 {
        return String::new();
    }

    let (body, footer) = split_stats_footer(output);
    let mut delta = String::new();
    delta.push_str(&delta_header(body));
    delta.push_str(
        "remem context changed since the previous injection. Compact delta shown; run `remem context --force` for a full refresh.\n\n",
    );
    let body_without_header = body
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or_default();
    delta.push_str(body_without_header.trim_start_matches('\n'));

    enforce_char_limit_preserving_footer(&mut delta, limit, footer);
    delta
}

fn split_stats_footer(output: &str) -> (&str, &str) {
    let Some(last_line_start) = output.trim_end_matches('\n').rfind('\n') else {
        return (output, "");
    };
    let footer_start = last_line_start + 1;
    let footer = &output[footer_start..];
    if super::is_context_stats_footer(footer.trim_end_matches('\n')) {
        (&output[..footer_start], footer)
    } else {
        (output, "")
    }
}

fn delta_header(output: &str) -> String {
    let first_line = output.lines().next().unwrap_or("# remem context");
    if let Some(context_idx) = first_line.find("] context ") {
        let mut header = String::new();
        header.push_str(&first_line[..context_idx]);
        header.push_str("] context delta ");
        header.push_str(&first_line[context_idx + "] context ".len()..]);
        header.push('\n');
        return header;
    }
    "# remem context delta\n".to_string()
}

fn enforce_char_limit_preserving_footer(output: &mut String, char_limit: usize, footer: &str) {
    if output.chars().count() <= char_limit {
        return;
    }

    let marker = "\n[remem context delta truncated]\n";
    let marker_chars = marker.chars().count();
    let footer_chars = footer.chars().count();

    if !footer.is_empty() && marker_chars + footer_chars < char_limit {
        let keep_chars = char_limit - marker_chars - footer_chars;
        let mut truncated: String = output.chars().take(keep_chars).collect();
        truncated.push_str(marker);
        truncated.push_str(footer);
        *output = truncated;
        return;
    }

    if marker_chars >= char_limit {
        *output = output.chars().take(char_limit).collect();
        return;
    }

    let keep_chars = char_limit - marker_chars;
    let mut truncated: String = output.chars().take(keep_chars).collect();
    truncated.push_str(marker);
    *output = truncated;
}

fn read_usize_env(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(default)
}
