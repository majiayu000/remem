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

fn find_last_open_tag_end(lowered: &str, tag: &str) -> Option<usize> {
    let mut search_from = 0;
    let mut last = None;
    while let Some(found) = find_open_tag_end(lowered, tag, search_from) {
        last = Some(found);
        search_from = found;
    }
    last
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

    let start = find_last_open_tag_end(&lowered, "summary")?;
    let end = find_close_tag_start(&lowered, "summary", start).unwrap_or(text.len());
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
