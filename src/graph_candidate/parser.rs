use anyhow::{bail, Context, Result};

use super::ParsedGraphCandidate;
use crate::memory::format::{extract_field, find_ascii_ci};

pub(super) fn parse_graph_candidates(text: &str) -> Result<Vec<ParsedGraphCandidate>> {
    let mut candidates = Vec::new();
    let mut pos = 0;
    while let Some(tag_start_rel) = find_ascii_ci(&text[pos..], "<graph_candidate") {
        let tag_start = pos + tag_start_rel;
        let Some(open_end_rel) = text[tag_start..].find('>') else {
            bail!("malformed graph_candidate output: unterminated opening tag");
        };
        let content_start = tag_start + open_end_rel + 1;
        let Some(close_rel) = find_ascii_ci(&text[content_start..], "</graph_candidate>") else {
            bail!("malformed graph_candidate output: missing closing tag");
        };
        let content_end = content_start + close_rel;
        candidates.push(parse_graph_candidate_content(
            &text[content_start..content_end],
        )?);
        pos = content_end + "</graph_candidate>".len();
    }
    Ok(candidates)
}

pub(super) fn parse_graph_defer_reason(text: &str) -> Option<String> {
    let tag_start = find_ascii_ci(text, "<defer")?;
    let open_end = text[tag_start..].find('>')?;
    let opening = &text[tag_start..tag_start + open_end + 1];
    graph_attr_value(opening, "reason")
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(str::to_string)
}

fn parse_graph_candidate_content(content: &str) -> Result<ParsedGraphCandidate> {
    let candidate_type =
        normalize_graph_candidate_type(graph_required_field(content, "type")?.as_str())?;
    let edge_type = normalize_graph_edge_type(
        &candidate_type,
        graph_required_field(content, "edge_type")?.as_str(),
    )?;
    let from_ref = normalize_graph_ref(graph_required_field(content, "from_ref")?.as_str())?;
    let to_ref = normalize_graph_ref(graph_required_field(content, "to_ref")?.as_str())?;
    let evidence_event_ids =
        parse_graph_evidence_ids(graph_required_field(content, "evidence_event_ids")?.as_str())?;
    let risk_class =
        normalize_graph_risk_class(graph_required_field(content, "risk_class")?.as_str())?;
    let confidence = parse_graph_confidence(graph_required_field(content, "confidence")?.as_str())?;
    let reason = normalize_graph_reason(graph_required_field(content, "reason")?.as_str())?;
    validate_graph_candidate_shape(&candidate_type, &edge_type, &to_ref)?;
    Ok(ParsedGraphCandidate {
        candidate_type,
        edge_type,
        from_ref,
        to_ref,
        evidence_event_ids,
        confidence,
        risk_class,
        reason,
    })
}

fn graph_required_field(content: &str, field: &str) -> Result<String> {
    extract_field(content, field)
        .with_context(|| format!("malformed graph_candidate output: missing <{field}>"))
}

fn normalize_graph_candidate_type(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "edge" => Ok(raw.trim().to_ascii_lowercase()),
        other => bail!("malformed graph_candidate output: invalid type '{other}'"),
    }
}

fn normalize_graph_edge_type(candidate_type: &str, raw: &str) -> Result<String> {
    let value = raw.trim().to_ascii_lowercase();
    match candidate_type {
        "edge" => match value.as_str() {
            "mentions" | "touches_file" | "conflicts" => Ok(value),
            other => bail!("malformed graph_candidate output: invalid edge_type '{other}'"),
        },
        _ => bail!("malformed graph_candidate output: invalid type '{candidate_type}'"),
    }
}

fn normalize_graph_ref(raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.is_empty() {
        bail!("malformed graph_candidate output: empty ref");
    }
    if value.len() > 512 {
        bail!("malformed graph_candidate output: ref too long");
    }
    if value.chars().any(|ch| ch == '\n' || ch == '\r') {
        bail!("malformed graph_candidate output: ref must be single-line");
    }
    let Some((prefix, rest)) = value.split_once(':') else {
        bail!("malformed graph_candidate output: ref must use '<kind>:<value>'");
    };
    if rest.trim().is_empty() {
        bail!("malformed graph_candidate output: empty ref value");
    }
    match prefix {
        "memory" | "entity" | "episode" | "file" | "state" | "claim" | "project" => {
            Ok(value.to_string())
        }
        other => bail!("malformed graph_candidate output: invalid ref kind '{other}'"),
    }
}

fn parse_graph_evidence_ids(raw: &str) -> Result<Vec<i64>> {
    let mut ids = raw
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<i64>()
                .with_context(|| "malformed graph_candidate output: invalid evidence_event_ids")
        })
        .collect::<Result<Vec<_>>>()?;
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        bail!("malformed graph_candidate output: empty evidence_event_ids");
    }
    if ids.iter().any(|id| *id <= 0) {
        bail!("malformed graph_candidate output: evidence_event_ids must be positive");
    }
    Ok(ids)
}

fn normalize_graph_risk_class(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "low" | "medium" | "high" => Ok(raw.trim().to_ascii_lowercase()),
        other => bail!("malformed graph_candidate output: invalid risk_class '{other}'"),
    }
}

fn parse_graph_confidence(raw: &str) -> Result<f64> {
    let confidence: f64 = raw
        .trim()
        .parse()
        .with_context(|| "malformed graph_candidate output: invalid confidence")?;
    if !(0.0..=1.0).contains(&confidence) {
        bail!("malformed graph_candidate output: confidence out of range");
    }
    Ok(confidence)
}

fn normalize_graph_reason(raw: &str) -> Result<String> {
    let reason = raw.trim();
    if reason.is_empty() {
        bail!("malformed graph_candidate output: empty reason");
    }
    Ok(reason.to_string())
}

fn validate_graph_candidate_shape(
    _candidate_type: &str,
    edge_type: &str,
    to_ref: &str,
) -> Result<()> {
    if edge_type == "touches_file" && !to_ref.starts_with("file:") {
        bail!("malformed graph_candidate output: touches_file to_ref must be file:<path>");
    }
    Ok(())
}

fn graph_attr_value<'a>(tag: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let value_start = tag.find(&needle)? + needle.len();
    let value_end = tag[value_start..].find('"')?;
    Some(&tag[value_start..value_start + value_end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_graph_candidates() -> Result<()> {
        let rows = parse_graph_candidates(
            "<graph_candidate><type>edge</type><edge_type>mentions</edge_type><from_ref>memory:1</from_ref><to_ref>entity:Worker</to_ref><evidence_event_ids>2,1,1</evidence_event_ids><risk_class>low</risk_class><confidence>0.91</confidence><reason>Observation names the worker.</reason></graph_candidate>",
        )?;

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].edge_type, "mentions");
        assert_eq!(rows[0].evidence_event_ids, vec![1, 2]);
        Ok(())
    }

    #[test]
    fn malformed_ref_fails_closed() {
        let err = parse_graph_candidates(
            "<graph_candidate><type>edge</type><edge_type>mentions</edge_type><from_ref>memory</from_ref><to_ref>entity:Worker</to_ref><evidence_event_ids>1</evidence_event_ids><risk_class>low</risk_class><confidence>0.91</confidence><reason>bad ref</reason></graph_candidate>",
        )
        .expect_err("bad ref should fail");

        assert!(err.to_string().contains("malformed graph_candidate"));
    }

    #[test]
    fn unpromotable_edge_type_fails_closed() {
        let err = parse_graph_candidates(
			"<graph_candidate><type>edge</type><edge_type>supports</edge_type><from_ref>memory:1</from_ref><to_ref>memory:2</to_ref><evidence_event_ids>1</evidence_event_ids><risk_class>low</risk_class><confidence>0.91</confidence><reason>unsupported edge</reason></graph_candidate>",
        )
        .expect_err("unsupported edge type should fail");

        assert!(err.to_string().contains("invalid edge_type 'supports'"));
    }

    #[test]
    fn unpromotable_candidate_type_fails_closed() {
        let result = parse_graph_candidates(
			"<graph_candidate><type>state_relation</type><edge_type>current_state</edge_type><from_ref>memory:1</from_ref><to_ref>state:active_focus</to_ref><evidence_event_ids>1</evidence_event_ids><risk_class>low</risk_class><confidence>0.91</confidence><reason>unsupported candidate type</reason></graph_candidate>",
		);
        let err = match result {
            Ok(_) => panic!("unsupported candidate type should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("invalid type 'state_relation'"));
    }

    #[test]
    fn episode_ref_is_valid_source_ref() -> Result<()> {
        let rows = parse_graph_candidates(
            "<graph_candidate><type>edge</type><edge_type>mentions</edge_type><from_ref>episode:42</from_ref><to_ref>entity:Worker</to_ref><evidence_event_ids>42</evidence_event_ids><risk_class>low</risk_class><confidence>0.91</confidence><reason>event mentions Worker.</reason></graph_candidate>",
        )?;

        assert_eq!(rows[0].from_ref, "episode:42");
        Ok(())
    }

    #[test]
    fn parses_graph_defer_reason_from_self_closing_tag() {
        assert_eq!(
            parse_graph_defer_reason("<defer reason=\"ambiguous alias\"/>"),
            Some("ambiguous alias".to_string())
        );
    }
}
