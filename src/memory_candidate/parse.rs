use anyhow::{bail, Context, Result};

use super::ParsedMemoryCandidate;
use crate::memory::format::{extract_field, find_ascii_ci};

pub(super) fn parse_memory_candidates(text: &str) -> Result<Vec<ParsedMemoryCandidate>> {
    let mut candidates = Vec::new();
    let mut pos = 0;
    while let Some(tag_start_rel) = find_ascii_ci(&text[pos..], "<memory_candidate") {
        let tag_start = pos + tag_start_rel;
        let Some(open_end_rel) = text[tag_start..].find('>') else {
            bail!("malformed memory_candidate output: unterminated opening tag");
        };
        let content_start = tag_start + open_end_rel + 1;
        let Some(close_rel) = find_ascii_ci(&text[content_start..], "</memory_candidate>") else {
            bail!("malformed memory_candidate output: missing closing tag");
        };
        let content_end = content_start + close_rel;
        candidates.push(parse_candidate_content(&text[content_start..content_end])?);
        pos = content_end + "</memory_candidate>".len();
    }
    Ok(candidates)
}

pub(super) fn parse_defer_reason(text: &str) -> Option<String> {
    let tag_start = find_ascii_ci(text, "<defer")?;
    let open_end = text[tag_start..].find('>')?;
    let opening = &text[tag_start..tag_start + open_end + 1];
    extract_attr(opening, "reason")
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(str::to_string)
}

fn parse_candidate_content(content: &str) -> Result<ParsedMemoryCandidate> {
    let scope = normalize_scope(required_field(content, "scope")?.as_str())?;
    let memory_type = normalize_memory_type(required_field(content, "type")?.as_str())?;
    let topic_key = normalize_topic_key(required_field(content, "topic_key")?.as_str())?;
    let risk_class = normalize_risk_class(required_field(content, "risk_class")?.as_str())?;
    let confidence = parse_confidence(required_field(content, "confidence")?.as_str())?;
    let text = required_field(content, "text")?;
    Ok(ParsedMemoryCandidate {
        scope,
        memory_type,
        topic_key,
        text,
        confidence,
        risk_class,
    })
}

fn required_field(content: &str, field: &str) -> Result<String> {
    extract_field(content, field)
        .with_context(|| format!("malformed memory_candidate output: missing <{field}>"))
}

pub(super) fn normalize_scope(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "global" | "workspace" | "project" => Ok(raw.trim().to_ascii_lowercase()),
        other => bail!("malformed memory_candidate output: invalid scope '{other}'"),
    }
}

pub(super) fn normalize_memory_type(raw: &str) -> Result<String> {
    let value = raw.trim().to_ascii_lowercase();
    match crate::memory::MemoryType::parse(&value) {
        Some(memory_type) if memory_type != crate::memory::MemoryType::SessionActivity => {
            Ok(memory_type.as_str().to_string())
        }
        _ => bail!("malformed memory_candidate output: invalid memory type '{value}'"),
    }
}

pub(super) fn normalize_topic_key(raw: &str) -> Result<String> {
    let value = crate::memory::slugify_for_topic(raw.trim(), 96);
    if value.is_empty() {
        bail!("malformed memory_candidate output: empty topic_key");
    }
    Ok(value)
}

fn normalize_risk_class(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "low" | "medium" | "high" => Ok(raw.trim().to_ascii_lowercase()),
        other => bail!("malformed memory_candidate output: invalid risk_class '{other}'"),
    }
}

fn parse_confidence(raw: &str) -> Result<f64> {
    let confidence: f64 = raw
        .trim()
        .parse()
        .with_context(|| "malformed memory_candidate output: invalid confidence")?;
    if !(0.0..=1.0).contains(&confidence) {
        bail!("malformed memory_candidate output: confidence out of range");
    }
    Ok(confidence)
}

fn extract_attr<'a>(tag: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let value_start = tag.find(&needle)? + needle.len();
    let value_end = tag[value_start..].find('"')?;
    Some(&tag[value_start..value_start + value_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defer_reason_from_self_closing_tag() {
        assert_eq!(
            parse_defer_reason("<defer reason=\"ambiguous conflict\"/>"),
            Some("ambiguous conflict".to_string())
        );
    }

    #[test]
    fn ignores_defer_without_reason() {
        assert_eq!(parse_defer_reason("<defer/>"), None);
    }
}
