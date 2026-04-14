fn is_open_tag_boundary(ch: u8) -> bool {
    ch.is_ascii_whitespace() || ch == b'>' || ch == b'/'
}

fn is_close_tag_boundary(ch: u8) -> bool {
    ch.is_ascii_whitespace() || ch == b'>'
}

pub(super) fn find_open_tag_end(lowered: &str, tag: &str, from: usize) -> Option<usize> {
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
            // Scan for the closing '>', skipping '>' inside quoted attribute values.
            let bytes = &lowered.as_bytes()[after_name..];
            let mut in_quote: Option<u8> = None;
            for (i, &b) in bytes.iter().enumerate() {
                match in_quote {
                    Some(q) if b == q => in_quote = None,
                    Some(_) => {}
                    None => match b {
                        b'"' | b'\'' => in_quote = Some(b),
                        b'>' => return Some(after_name + i + 1),
                        _ => {}
                    },
                }
            }
            return None;
        }
        return None;
    }
    None
}

pub(super) fn find_close_tag_start(lowered: &str, tag: &str, from: usize) -> Option<usize> {
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

pub(super) fn find_close_tag_end(lowered: &str, tag: &str, from: usize) -> Option<usize> {
    let close_start = find_close_tag_start(lowered, tag, from)?;
    let after_name = close_start + 2 + tag.len();
    lowered[after_name..]
        .find('>')
        .map(|end_rel| after_name + end_rel + 1)
}

pub(super) fn fallback_value_end(content: &str, lowered: &str, start: usize) -> usize {
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

pub fn extract_field(content: &str, field: &str) -> Option<String> {
    let lowered = content.to_ascii_lowercase();
    let start = find_open_tag_end(&lowered, field, 0)?;
    let end = find_close_tag_start(&lowered, field, start)
        .unwrap_or_else(|| fallback_value_end(content, &lowered, start));
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
