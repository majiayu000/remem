//! Topic Loom segment parsing + persistence for session rollup (Phase 1.3).
//!
//! The rollup LLM call emits, in addition to `<summary>`, a `<segments>` block:
//! ```xml
//! <segments>
//!   <segment>
//!     <topic_key>fts5-tokenizer</topic_key>
//!     <status>resolved</status>
//!     <title>...</title>
//!     <summary>...</summary>
//!     <from_event_id>100</from_event_id>
//!     <to_event_id>110</to_event_id>
//!     <files>a.rs, b.rs</files>
//!   </segment>
//! </segments>
//! ```
//! All-child-tag form so we reuse `memory::format::extract_field`. Segments are
//! overlap-tolerant (Phase 0); `(from,to)` must stay within the rollup range.

use anyhow::Result;
use rusqlite::Connection;

use crate::db::{self, TopicSegmentInput};
use crate::memory::format::{extract_field, find_ascii_ci};

const VALID_STATUS: [&str; 3] = ["open", "resolved", "superseded"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSegment {
    pub segment_index: i64,
    pub topic_key: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub from_event_id: i64,
    pub to_event_id: i64,
    pub files: Vec<String>,
}

/// Parse `<segment>` blocks. Drops (with a warning) any segment missing a
/// topic_key / event ids, or whose `(from,to)` falls outside the rollup range.
pub(crate) fn parse_segments(text: &str, valid_from: i64, valid_to: i64) -> Vec<ParsedSegment> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut index = 0i64;

    while let Some(rel) = find_ascii_ci(&text[pos..], "<segment>") {
        let content_start = pos + rel + "<segment>".len();
        let Some(close_rel) = find_ascii_ci(&text[content_start..], "</segment>") else {
            break;
        };
        let content_end = content_start + close_rel;
        let content = &text[content_start..content_end];
        pos = content_end + "</segment>".len();

        let Some(topic_key) = extract_field(content, "topic_key") else {
            crate::log::warn("topic-segments", "dropped segment: missing topic_key");
            continue;
        };
        let from = extract_field(content, "from_event_id").and_then(|s| s.parse::<i64>().ok());
        let to = extract_field(content, "to_event_id").and_then(|s| s.parse::<i64>().ok());
        let (Some(from), Some(to)) = (from, to) else {
            crate::log::warn(
                "topic-segments",
                &format!("dropped segment topic_key={topic_key}: missing/invalid event ids"),
            );
            continue;
        };
        if from > to || from < valid_from || to > valid_to {
            crate::log::warn(
                "topic-segments",
                &format!(
                    "dropped segment topic_key={topic_key}: range {from}..{to} outside {valid_from}..{valid_to}"
                ),
            );
            continue;
        }

        let status = extract_field(content, "status").unwrap_or_else(|| "open".to_string());
        let status = if VALID_STATUS.contains(&status.as_str()) {
            status
        } else {
            "open".to_string()
        };
        let files = extract_field(content, "files")
            .map(|raw| {
                raw.split(',')
                    .map(|f| f.trim().to_string())
                    .filter(|f| !f.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        out.push(ParsedSegment {
            segment_index: index,
            topic_key,
            title: extract_field(content, "title").unwrap_or_default(),
            summary: extract_field(content, "summary").unwrap_or_default(),
            status,
            from_event_id: from,
            to_event_id: to,
            files,
        });
        index += 1;
    }
    out
}

/// Persist parsed segments. Idempotent per (session_row_id, topic_key): a
/// segment whose topic_key already exists for this session is skipped.
pub(crate) fn persist_segments(
    conn: &Connection,
    host_id: i64,
    project_id: i64,
    session_row_id: i64,
    project: &str,
    segments: &[ParsedSegment],
) -> Result<usize> {
    let mut inserted = 0usize;
    for seg in segments {
        if db::topic_segment_exists(conn, session_row_id, &seg.topic_key)? {
            continue;
        }
        let evidence = serde_json::to_string(&[seg.from_event_id, seg.to_event_id])?;
        let files_json = if seg.files.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&seg.files)?)
        };
        db::insert_topic_segment(
            conn,
            &TopicSegmentInput {
                host_id,
                project_id,
                session_row_id,
                project,
                topic_key: &seg.topic_key,
                title: &seg.title,
                summary: &seg.summary,
                status: &seg.status,
                segment_index: seg.segment_index,
                covered_from_event_id: seg.from_event_id,
                covered_to_event_id: seg.to_event_id,
                evidence_event_ids: &evidence,
                files: files_json.as_deref(),
                confidence: 0.75,
            },
        )?;
        inserted += 1;
    }
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
<summary>overall</summary>
<segments>
  <segment>
    <topic_key>anti-bot-research</topic_key>
    <status>resolved</status>
    <title>Anti-bot tooling</title>
    <summary>researched blocked sources</summary>
    <from_event_id>3056</from_event_id>
    <to_event_id>3466</to_event_id>
    <files>docs/a.md</files>
  </segment>
  <segment>
    <topic_key>kexue-scraping</topic_key>
    <status>resolved</status>
    <title>Kexue scraping</title>
    <summary>wayback CDX</summary>
    <from_event_id>3057</from_event_id>
    <to_event_id>3331</to_event_id>
  </segment>
</segments>";

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::migrate::run_migrations(&conn).expect("migrations");
        conn.execute_batch("PRAGMA foreign_keys=OFF;")
            .expect("disable foreign keys");
        conn
    }

    #[test]
    fn parses_overlapping_segments() {
        let segs = parse_segments(SAMPLE, 3000, 3500);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].topic_key, "anti-bot-research");
        assert_eq!(segs[0].segment_index, 0);
        assert_eq!(segs[0].files, vec!["docs/a.md".to_string()]);
        // Phase 0: the two ranges overlap/nest — both must survive parsing.
        assert_eq!(segs[1].topic_key, "kexue-scraping");
        assert!(segs[1].from_event_id < segs[0].to_event_id);
    }

    #[test]
    fn drops_out_of_range_and_missing_key() {
        let xml = "\
<segments>
  <segment><status>open</status><from_event_id>10</from_event_id><to_event_id>20</to_event_id></segment>
  <segment><topic_key>too-late</topic_key><from_event_id>10</from_event_id><to_event_id>999</to_event_id></segment>
  <segment><topic_key>ok</topic_key><from_event_id>10</from_event_id><to_event_id>20</to_event_id></segment>
</segments>";
        let segs = parse_segments(xml, 5, 100);
        assert_eq!(segs.len(), 1, "missing-key and out-of-range segments dropped");
        assert_eq!(segs[0].topic_key, "ok");
    }

    #[test]
    fn persist_is_idempotent_per_topic_key() -> Result<()> {
        let conn = conn();
        let segs = parse_segments(SAMPLE, 3000, 3500);
        let first = persist_segments(&conn, 1, 1, 7, "/tmp/remem", &segs)?;
        assert_eq!(first, 2);
        let second = persist_segments(&conn, 1, 1, 7, "/tmp/remem", &segs)?;
        assert_eq!(second, 0, "same topic_keys must not be re-inserted");
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM topic_segments WHERE session_row_id = 7",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 2);
        Ok(())
    }
}
