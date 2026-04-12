use super::{
    extract::{fallback_value_end, find_close_tag_end, find_close_tag_start, find_open_tag_end},
    extract_field, ParsedObservation, OBSERVATION_TYPES,
};

/// Find `needle` in `haystack` using ASCII case-insensitive comparison.
/// Returns the byte offset of the first match, or `None`.
fn find_ascii_ci(haystack: &str, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

fn extract_array(content: &str, array_name: &str, element_name: &str) -> Vec<String> {
    let lowered = content.to_ascii_lowercase();
    let Some(start) = find_open_tag_end(&lowered, array_name, 0) else {
        return vec![];
    };
    let end = find_close_tag_start(&lowered, array_name, start).unwrap_or(content.len());
    let inner = &content[start..end];
    let inner_lower = inner.to_ascii_lowercase();

    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(value_start) = find_open_tag_end(&inner_lower, element_name, pos) {
        let value_end = find_close_tag_start(&inner_lower, element_name, value_start)
            .unwrap_or_else(|| fallback_value_end(inner, &inner_lower, value_start));
        if value_start >= value_end {
            pos = value_start.saturating_add(1);
            continue;
        }
        let value = inner[value_start..value_end].trim().to_string();
        if !value.is_empty() {
            results.push(value);
        }
        pos = find_close_tag_end(&inner_lower, element_name, value_start).unwrap_or(value_end);
    }
    results
}

pub fn parse_observations(text: &str) -> Vec<ParsedObservation> {
    let mut observations = Vec::new();
    let mut pos = 0;

    while let Some(tag_start_rel) = find_ascii_ci(&text[pos..], "<observation") {
        let tag_start = pos + tag_start_rel;
        let Some(open_end_rel) = text[tag_start..].find('>') else {
            break;
        };
        let content_start = tag_start + open_end_rel + 1;
        let close_tag = "</observation>";
        let close_rel = find_ascii_ci(&text[content_start..], close_tag);
        let content_end = close_rel
            .map(|rel| content_start + rel)
            .unwrap_or(text.len());
        let content = &text[content_start..content_end];

        let raw_type = extract_field(content, "type").unwrap_or_default();
        let obs_type = if OBSERVATION_TYPES.contains(&raw_type.as_str()) {
            raw_type
        } else {
            "discovery".to_string()
        };

        let mut concepts = extract_array(content, "concepts", "concept");
        concepts.retain(|concept| concept != &obs_type);

        observations.push(ParsedObservation {
            obs_type,
            title: extract_field(content, "title"),
            subtitle: extract_field(content, "subtitle"),
            facts: extract_array(content, "facts", "fact"),
            narrative: extract_field(content, "narrative"),
            concepts,
            files_read: extract_array(content, "files_read", "file"),
            files_modified: extract_array(content, "files_modified", "file"),
        });

        pos = if close_rel.is_some() {
            content_end + close_tag.len()
        } else {
            text.len()
        };
    }

    observations
}
