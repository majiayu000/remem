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
        title_override: None,
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
    // Candidate 词汇表：decision/discovery/bugfix/architecture/lesson/preference/procedure
    // (session_activity 不是合法 candidate type)。
    if let Some(memory_type) = crate::memory::MemoryType::parse(&value) {
        if memory_type != crate::memory::MemoryType::SessionActivity {
            return Ok(memory_type.as_str().to_string());
        }
    }
    // `fact` is an intuitive candidate label that LLMs emit for factual
    // discoveries, but it is not part of either canonical vocabulary.
    if value == "fact" {
        return Ok(crate::memory::MemoryType::Discovery.as_str().to_string());
    }
    // LLM 经常把 observation type（feature/refactor/change/discovery/bugfix/decision）
    // 误抄进 <type>。observation 与 candidate 是两套词汇表，复用 from_observation_type
    // 归一映射，避免单个误抄值让整批 candidate 被 bail 拖死。
    if let Some(mapped) = crate::memory::MemoryType::from_observation_type(&value) {
        return Ok(mapped.as_str().to_string());
    }
    bail!("malformed memory_candidate output: invalid memory type '{value}'")
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

    #[test]
    fn normalizes_canonical_candidate_types() {
        for memory_type in crate::memory::MemoryType::ALL
            .iter()
            .copied()
            .filter(|memory_type| *memory_type != crate::memory::MemoryType::SessionActivity)
        {
            assert_eq!(
                normalize_memory_type(memory_type.as_str()).unwrap(),
                memory_type.as_str()
            );
        }
    }

    #[test]
    fn maps_observation_type_vocab_to_candidate_type() {
        // LLM 把 observation type 误抄进 <type>；归一到对应 candidate type，而非 bail 整批。
        assert_eq!(normalize_memory_type("feature").unwrap(), "discovery");
        assert_eq!(normalize_memory_type("refactor").unwrap(), "discovery");
        assert_eq!(normalize_memory_type("change").unwrap(), "discovery");
        assert_eq!(normalize_memory_type("bugfix").unwrap(), "bugfix");
        assert_eq!(normalize_memory_type("decision").unwrap(), "decision");
    }

    #[test]
    fn maps_fact_alias_without_dropping_neighbor_candidate() {
        assert_eq!(normalize_memory_type(" Fact ").unwrap(), "discovery");

        let parsed = parse_memory_candidates(
            "<memory_candidate><scope>project</scope><type>fact</type><topic_key>worker-model</topic_key><risk_class>low</risk_class><confidence>0.91</confidence><text>The worker uses one writer connection.</text></memory_candidate>\
             <memory_candidate><scope>project</scope><type>decision</type><topic_key>retry-policy</topic_key><risk_class>medium</risk_class><confidence>0.88</confidence><text>Retry malformed extraction output.</text></memory_candidate>",
        )
        .unwrap();

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].memory_type, "discovery");
        assert_eq!(parsed[1].memory_type, "decision");
    }

    #[test]
    fn rejects_non_candidate_memory_types() {
        // session_activity 是合法 MemoryType 但不是 candidate type
        assert!(normalize_memory_type("session_activity").is_err());
        // status 是 LLM 自由发挥值；保持拒绝比错误归一化更安全
        assert!(normalize_memory_type("status").is_err());
        assert!(normalize_memory_type("nonsense").is_err());
    }
}
