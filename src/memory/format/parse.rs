use super::{extract_field, ParsedObservation, OBSERVATION_TYPES};

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
    let open = format!("<{}>", array_name);
    let close = format!("</{}>", array_name);
    let Some(start) = content.find(&open) else {
        return vec![];
    };
    let start = start + open.len();
    let Some(end_rel) = content[start..].find(&close) else {
        return vec![];
    };
    let end = start + end_rel;
    let inner = &content[start..end];

    let elem_open = format!("<{}>", element_name);
    let elem_close = format!("</{}>", element_name);
    let mut results = Vec::new();
    let mut pos = 0;
    while let Some(found) = inner[pos..].find(&elem_open) {
        let value_start = pos + found + elem_open.len();
        let Some(end_rel) = inner[value_start..].find(&elem_close) else {
            break;
        };
        let value_end = value_start + end_rel;
        let value = inner[value_start..value_end].trim().to_string();
        if !value.is_empty() {
            results.push(value);
        }
        pos = value_end + elem_close.len();
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
        let Some(close_rel) = find_ascii_ci(&text[content_start..], "</observation>") else {
            break;
        };
        let content_end = content_start + close_rel;
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

        pos = content_end + "</observation>".len();
    }

    observations
}
