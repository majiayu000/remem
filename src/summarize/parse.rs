pub struct ParsedSummary {
    pub request: Option<String>,
    pub completed: Option<String>,
    pub decisions: Option<String>,
    pub learned: Option<String>,
    pub next_steps: Option<String>,
    pub preferences: Option<String>,
}

fn is_open_tag_boundary(ch: u8) -> bool {
    ch.is_ascii_whitespace() || ch == b'>' || ch == b'/'
}

fn is_close_tag_boundary(ch: u8) -> bool {
    ch.is_ascii_whitespace() || ch == b'>'
}

fn find_open_tag_end(lowered: &str, tag: &str, from: usize) -> Option<usize> {
    let needle = format!("<{}", tag);
    let mut search_from = from;
    while let Some(rel) = lowered[search_from..].find(&needle) {
        let start = search_from + rel;
        let after_name = start + needle.len();
        if let Some(&boundary) = lowered.as_bytes().get(after_name) {
            if !is_open_tag_boundary(boundary) {
                search_from = after_name;
                continue;
            }
            return lowered[after_name..]
                .find('>')
                .map(|close_rel| after_name + close_rel + 1);
        }
        return None;
    }
    None
}

fn find_close_tag_start(lowered: &str, tag: &str, from: usize) -> Option<usize> {
    let needle = format!("</{}", tag);
    let mut search_from = from;
    while let Some(rel) = lowered[search_from..].find(&needle) {
        let start = search_from + rel;
        let after_name = start + needle.len();
        if let Some(&boundary) = lowered.as_bytes().get(after_name) {
            if !is_close_tag_boundary(boundary) {
                search_from = after_name;
                continue;
            }
        }
        return Some(start);
    }
    None
}

/// Find the last complete `<summary>...</summary>` block, or fall back to the last
/// open tag when the final block is truncated (no close tag).
///
/// When multiple open tags share the same close tag (e.g. a literal `<summary>` inside
/// a field), the *first* open tag of each close-group is kept so the outer wrapper is
/// not discarded in favour of an embedded token.
fn find_last_summary_range(lowered: &str, text_len: usize) -> Option<(usize, usize)> {
    let mut search_from = 0;
    let mut candidates: Vec<(usize, usize)> = Vec::new();
    let mut last_open: Option<usize> = None;

    while let Some(open_end) = find_open_tag_end(lowered, "summary", search_from) {
        last_open = Some(open_end);
        if let Some(close_pos) = find_close_tag_start(lowered, "summary", open_end) {
            candidates.push((open_end, close_pos));
        }
        search_from = open_end;
    }

    // Walk candidates; for each unique close tag keep only the *first* open of that group,
    // then take the last such group — this is the last genuinely-complete summary block.
    let mut last_pair: Option<(usize, usize)> = None;
    let mut prev_close: Option<usize> = None;
    for &(open, close) in &candidates {
        if prev_close != Some(close) {
            last_pair = Some((open, close));
            prev_close = Some(close);
        }
        // Same close as the previous entry — already recorded the earlier open for this group.
    }

    if let Some((comp_start, comp_end)) = last_pair {
        // If there is a trailing open tag without a matching close, prefer it (truncated block).
        if let Some(last_start) = last_open {
            if last_start > comp_start {
                return Some((last_start, text_len));
            }
        }
        return Some((comp_start, comp_end));
    }

    // No complete pair found — use the last open tag (truncated wrapper).
    last_open.map(|start| (start, text_len))
}

fn is_recovery_boundary_tag(tag: &str) -> bool {
    matches!(
        tag,
        "request"
            | "completed"
            | "decisions"
            | "learned"
            | "next_steps"
            | "preferences"
            | "summary"
            | "skip_summary"
    )
}

fn fallback_value_end(content: &str, lowered: &str, start: usize) -> usize {
    let bytes = lowered.as_bytes();
    let mut search_from = start;
    while let Some(rel) = lowered[search_from..].find('<') {
        let next_tag = search_from + rel;
        let tag_start = match bytes.get(next_tag + 1) {
            Some(b'/') => {
                search_from = next_tag + 2;
                continue;
            }
            Some(ch) if ch.is_ascii_alphabetic() => next_tag + 1,
            _ => {
                search_from = next_tag + 1;
                continue;
            }
        };
        let tag_end = bytes[tag_start..]
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
            .map(|rel_end| tag_start + rel_end)
            .unwrap_or(content.len());
        if is_recovery_boundary_tag(&lowered[tag_start..tag_end]) {
            return next_tag;
        }
        search_from = next_tag + 1;
    }
    content.len()
}

fn extract_field_relaxed(content: &str, field: &str) -> Option<String> {
    let lowered = content.to_ascii_lowercase();
    let start = find_open_tag_end(&lowered, field, 0)?;
    let fallback_end = fallback_value_end(content, &lowered, start);
    let end = match find_close_tag_start(&lowered, field, start) {
        // Close tag exists but comes after the first sibling open tag — likely
        // misplaced inside another field's content; use the fallback boundary.
        Some(close_start) if fallback_end < close_start => fallback_end,
        Some(close_start) => close_start,
        None => fallback_end,
    };
    if start >= end {
        return None;
    }
    let value = content[start..end].trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn parse_summary(text: &str) -> Option<ParsedSummary> {
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("<skip_summary") {
        return None;
    }

    let (start, end) = find_last_summary_range(&lowered, text.len())?;
    let content = &text[start..end];

    Some(ParsedSummary {
        request: extract_field_relaxed(content, "request"),
        completed: extract_field_relaxed(content, "completed"),
        decisions: extract_field_relaxed(content, "decisions"),
        learned: extract_field_relaxed(content, "learned"),
        next_steps: extract_field_relaxed(content, "next_steps"),
        preferences: extract_field_relaxed(content, "preferences"),
    })
}
