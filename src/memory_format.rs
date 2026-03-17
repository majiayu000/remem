pub const OBSERVATION_TYPES: &[&str] = &[
    "bugfix",
    "feature",
    "refactor",
    "discovery",
    "decision",
    "change",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedObservation {
    pub obs_type: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub facts: Vec<String>,
    pub narrative: Option<String>,
    pub concepts: Vec<String>,
    pub files_read: Vec<String>,
    pub files_modified: Vec<String>,
}

pub fn xml_escape_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn xml_escape_attr(raw: &str) -> String {
    xml_escape_text(raw)
}

pub fn extract_field(content: &str, field: &str) -> Option<String> {
    let open = format!("<{}>", field);
    let close = format!("</{}>", field);
    let start = content.find(&open)? + open.len();
    let end_rel = content[start..].find(&close)?;
    let end = start + end_rel;
    if start >= end {
        return None;
    }
    let val = content[start..end].trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
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
    while let Some(s) = inner[pos..].find(&elem_open) {
        let val_start = pos + s + elem_open.len();
        if let Some(e_rel) = inner[val_start..].find(&elem_close) {
            let val_end = val_start + e_rel;
            let val = inner[val_start..val_end].trim().to_string();
            if !val.is_empty() {
                results.push(val);
            }
            pos = val_end + elem_close.len();
        } else {
            break;
        }
    }
    results
}

pub fn parse_observations(text: &str) -> Vec<ParsedObservation> {
    let mut observations = Vec::new();
    let mut pos = 0;

    while let Some(tag_start_rel) = text[pos..].find("<observation") {
        let tag_start = pos + tag_start_rel;
        let Some(open_end_rel) = text[tag_start..].find('>') else {
            break;
        };
        let content_start = tag_start + open_end_rel + 1;
        let Some(close_rel) = text[content_start..].find("</observation>") else {
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
        concepts.retain(|c| c != &obs_type);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_field_scans_from_open_tag() {
        let body = "</title><title>ok</title>";
        assert_eq!(extract_field(body, "title").as_deref(), Some("ok"));
    }

    #[test]
    fn xml_escape_escapes_angle_and_amp() {
        assert_eq!(xml_escape_text(r#"a<&>"'"#), "a&lt;&amp;&gt;&quot;&apos;");
    }
}
