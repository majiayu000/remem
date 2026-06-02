use std::collections::HashSet;

use anyhow::{anyhow, Context, Result};

use super::RollupRange;

#[derive(Debug, Clone)]
pub(super) struct RollupOutput {
    pub(super) summary_text: String,
    pub(super) segments: Vec<ParsedTopicSegment>,
}

#[derive(Debug, Clone)]
pub(super) struct ParsedTopicSegment {
    pub(super) topic_key: String,
    pub(super) title: String,
    pub(super) summary: String,
    pub(super) status: String,
    pub(super) segment_index: i64,
    pub(super) covered_from_event_id: i64,
    pub(super) covered_to_event_id: i64,
    pub(super) evidence_event_ids: Vec<i64>,
    pub(super) files: Vec<String>,
    pub(super) confidence: f64,
}

pub(super) fn parse_rollup_response(text: &str, range: &RollupRange) -> Result<RollupOutput> {
    let summary_text = extract_tag(text, "summary")
        .map(|summary| summary.trim().to_string())
        .filter(|summary| !summary.is_empty())
        .ok_or_else(|| anyhow!("session_rollup response missing non-empty <summary>"))?;

    let Some(segments_xml) = extract_tag(text, "segments") else {
        crate::log::info(
            "session_rollup",
            "response has no <segments>; writing summary only",
        );
        return Ok(RollupOutput {
            summary_text,
            segments: Vec::new(),
        });
    };

    let mut segments = Vec::new();
    for (index, raw_segment) in iter_segment_blocks(&segments_xml).into_iter().enumerate() {
        match parse_segment(index as i64, &raw_segment, range) {
            Ok(Some(segment)) => segments.push(segment),
            Ok(None) => {}
            Err(error) => crate::log::warn(
                "session_rollup",
                &format!("dropping invalid topic segment: {error}"),
            ),
        }
    }
    if segments.is_empty() {
        crate::log::info(
            "session_rollup",
            "response produced no valid topic segments",
        );
    }
    Ok(RollupOutput {
        summary_text,
        segments,
    })
}

fn iter_segment_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start_rel) = rest.find("<segment") {
        let after_start = &rest[start_rel..];
        let Some(open_end_rel) = after_start.find('>') else {
            break;
        };
        let body_start = start_rel + open_end_rel + 1;
        let Some(close_rel) = rest[body_start..].find("</segment>") else {
            break;
        };
        let close_end = body_start + close_rel + "</segment>".len();
        blocks.push(rest[start_rel..close_end].to_string());
        rest = &rest[close_end..];
    }
    blocks
}

fn parse_segment(
    segment_index: i64,
    raw_segment: &str,
    range: &RollupRange,
) -> Result<Option<ParsedTopicSegment>> {
    let open_end = raw_segment
        .find('>')
        .context("segment missing opening tag terminator")?;
    let open_tag = &raw_segment[..=open_end];
    let body = &raw_segment[open_end + 1..raw_segment.len() - "</segment>".len()];

    let topic_key = extract_attr(open_tag, "topic_key")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .context("segment missing topic_key")?;
    if !is_valid_topic_key(&topic_key) {
        return Err(anyhow!("invalid topic_key '{topic_key}'"));
    }

    let status = extract_attr(open_tag, "status").unwrap_or_else(|| "open".to_string());
    if !matches!(status.as_str(), "open" | "resolved" | "superseded") {
        return Err(anyhow!("invalid segment status '{status}'"));
    }

    let title = required_tag(body, "title")?;
    let summary = required_tag(body, "summary")?;
    let explicit_from = required_i64(body, "from_event_id")?;
    let explicit_to = required_i64(body, "to_event_id")?;
    if explicit_to < explicit_from {
        return Err(anyhow!(
            "to_event_id {} is before from_event_id {}",
            explicit_to,
            explicit_from
        ));
    }

    let mut evidence_event_ids = extract_tag(body, "evidence_event_ids")
        .map(|raw| parse_event_ids(&raw))
        .transpose()?
        .unwrap_or_else(|| vec![explicit_from, explicit_to]);
    evidence_event_ids.sort_unstable();
    evidence_event_ids.dedup();
    if evidence_event_ids.is_empty() {
        return Err(anyhow!("segment has empty evidence_event_ids"));
    }
    let loaded_event_ids = range
        .events
        .iter()
        .map(|event| event.id)
        .collect::<HashSet<_>>();
    let missing_event_ids = evidence_event_ids
        .iter()
        .copied()
        .filter(|event_id| !loaded_event_ids.contains(event_id))
        .collect::<Vec<_>>();
    if !missing_event_ids.is_empty() {
        return Err(anyhow!(
            "evidence_event_ids absent from loaded rollup events: {:?}",
            missing_event_ids
        ));
    }
    let covered_from_event_id = evidence_event_ids[0];
    let covered_to_event_id = evidence_event_ids[evidence_event_ids.len() - 1];

    let files = extract_tag(body, "files")
        .map(|raw| parse_files(&raw))
        .transpose()?
        .unwrap_or_default();
    let confidence = extract_attr(open_tag, "confidence")
        .as_deref()
        .map(str::parse::<f64>)
        .transpose()
        .context("parse segment confidence")?
        .unwrap_or(0.75);

    Ok(Some(ParsedTopicSegment {
        topic_key,
        title,
        summary,
        status,
        segment_index,
        covered_from_event_id,
        covered_to_event_id,
        evidence_event_ids,
        files,
        confidence,
    }))
}

fn required_tag(body: &str, tag: &str) -> Result<String> {
    extract_tag(body, tag)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("segment missing non-empty <{tag}>"))
}

fn required_i64(body: &str, tag: &str) -> Result<i64> {
    required_tag(body, tag)?
        .parse::<i64>()
        .with_context(|| format!("parse <{tag}>"))
}

fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(xml_unescape_text(text[start..end].trim()))
}

fn extract_attr(open_tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let start = open_tag.find(&needle)? + needle.len();
    let end = open_tag[start..].find('"')? + start;
    Some(xml_unescape_text(&open_tag[start..end]))
}

fn parse_event_ids(raw: &str) -> Result<Vec<i64>> {
    if let Ok(ids) = serde_json::from_str::<Vec<i64>>(raw.trim()) {
        return Ok(ids);
    }
    raw.split(|ch: char| ch == ',' || ch.is_whitespace())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<i64>()
                .with_context(|| format!("parse evidence event id '{part}'"))
        })
        .collect()
}

fn parse_files(raw: &str) -> Result<Vec<String>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    if let Ok(files) = serde_json::from_str::<Vec<String>>(raw.trim()) {
        return Ok(files);
    }
    let mut files = raw
        .split([',', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    Ok(files)
}

fn is_valid_topic_key(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn xml_unescape_text(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_rollup::{RollupEvent, RollupRange};

    fn event(id: i64) -> RollupEvent {
        RollupEvent {
            id,
            event_type: "tool_result".to_string(),
            role: None,
            tool_name: None,
            content: format!("content {id}"),
            token_estimate: 1,
            created_at_epoch: 100 + id,
            turn_id: None,
        }
    }

    fn range() -> RollupRange {
        RollupRange {
            from_event_id: 10,
            to_event_id: 20,
            events: vec![event(10), event(11), event(14), event(20)],
        }
    }

    #[test]
    fn parses_segments_with_overlapping_event_ranges() -> Result<()> {
        let parsed = parse_rollup_response(
            r#"<summary>done</summary>
            <segments>
            <segment topic_key="anti-bot-research" status="resolved">
              <title>Anti-bot research</title>
              <summary>Investigated blocking.</summary>
              <evidence_event_ids>10,14,20</evidence_event_ids>
              <from_event_id>10</from_event_id>
              <to_event_id>20</to_event_id>
              <files>src/a.rs,src/b.rs</files>
            </segment>
            <segment topic_key="kexue-scraping" status="open">
              <title>Kexue scraping</title>
              <summary>Implemented scraper.</summary>
              <evidence_event_ids>[11, 14]</evidence_event_ids>
              <from_event_id>11</from_event_id>
              <to_event_id>14</to_event_id>
            </segment>
            </segments>"#,
            &range(),
        )?;

        assert_eq!(parsed.summary_text, "done");
        assert_eq!(parsed.segments.len(), 2);
        assert_eq!(parsed.segments[0].covered_from_event_id, 10);
        assert_eq!(parsed.segments[0].covered_to_event_id, 20);
        assert_eq!(parsed.segments[1].evidence_event_ids, vec![11, 14]);
        Ok(())
    }

    #[test]
    fn drops_segment_with_evidence_event_absent_from_loaded_events() -> Result<()> {
        let parsed = parse_rollup_response(
            r#"<summary>done</summary>
            <segments>
            <segment topic_key="interleaved-session" status="open">
              <title>Interleaved session</title>
              <summary>Should not attach unrelated evidence.</summary>
              <evidence_event_ids>10,15,20</evidence_event_ids>
              <from_event_id>10</from_event_id>
              <to_event_id>20</to_event_id>
            </segment>
            </segments>"#,
            &range(),
        )?;

        assert!(parsed.segments.is_empty());
        Ok(())
    }

    #[test]
    fn missing_summary_fails_entire_rollup_parse() {
        let err = parse_rollup_response("<segments></segments>", &range())
            .expect_err("missing summary should fail");
        assert!(err.to_string().contains("missing non-empty <summary>"));
    }
}
