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

fn fallback_value_end(content: &str, lowered: &str, start: usize) -> usize {
    let mut next_tag = content[start..]
        .char_indices()
        .find_map(|(idx, ch)| (ch == '<').then_some(start + idx))
        .unwrap_or(content.len());
    while next_tag < content.len() && lowered[next_tag..].starts_with("</") {
        let after = next_tag + 2;
        next_tag = content[after..]
            .char_indices()
            .find_map(|(idx, ch)| (ch == '<').then_some(after + idx))
            .unwrap_or(content.len());
        if next_tag == content.len() {
            break;
        }
    }
    next_tag
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
