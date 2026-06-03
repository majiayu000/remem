use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// A coherent topic segment produced during session rollup (Topic Loom).
///
/// Segments are overlap-tolerant: parallel/interleaved work makes their event
/// ranges nest, so `evidence_event_ids` (a JSON array) is the authoritative
/// link to source events, while `covered_from/to_event_id` are derived min/max
/// used only for ordering and range queries. See SPEC-topic-continuity.md §0/§4.1.
pub struct TopicSegmentInput<'a> {
    pub host_id: i64,
    pub project_id: i64,
    pub session_row_id: i64,
    pub project: &'a str,
    pub topic_key: &'a str,
    pub title: &'a str,
    pub summary: &'a str,
    pub status: &'a str,
    pub segment_index: i64,
    pub covered_from_event_id: i64,
    pub covered_to_event_id: i64,
    pub evidence_event_ids: &'a str,
    pub files: Option<&'a str>,
    pub confidence: f64,
}

pub fn insert_topic_segment(conn: &Connection, seg: &TopicSegmentInput) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO topic_segments \
         (host_id, project_id, session_row_id, project, topic_key, title, summary, \
          status, segment_index, covered_from_event_id, covered_to_event_id, \
          evidence_event_ids, files, confidence, created_at_epoch, updated_at_epoch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
        params![
            seg.host_id,
            seg.project_id,
            seg.session_row_id,
            seg.project,
            seg.topic_key,
            seg.title,
            seg.summary,
            seg.status,
            seg.segment_index,
            seg.covered_from_event_id,
            seg.covered_to_event_id,
            seg.evidence_event_ids,
            seg.files,
            seg.confidence,
            now
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Idempotency guard for rollup persistence: has a segment for this
/// (session, topic_key) already been written? Keyed on topic_key — never on
/// the (from,to) range, which overlaps across interleaved topics (Phase 0).
pub fn topic_segment_exists(
    conn: &Connection,
    session_row_id: i64,
    topic_key: &str,
) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM topic_segments \
             WHERE session_row_id = ?1 AND topic_key = ?2 LIMIT 1",
            params![session_row_id, topic_key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// One segment row in a topic trace (Trace Weaver, read side).
#[derive(Debug, Clone)]
pub struct TopicSegmentRow {
    pub id: i64,
    pub session_row_id: i64,
    pub topic_key: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub covered_from_event_id: i64,
    pub covered_to_event_id: i64,
    pub created_at_epoch: i64,
}

/// Load every segment for a topic in one project, time-ordered into a trace.
/// Dynamic aggregation — no materialized table (SPEC §4.2 Trace Weaver).
pub fn load_trace_by_topic_key(
    conn: &Connection,
    project: &str,
    topic_key: &str,
) -> Result<Vec<TopicSegmentRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, session_row_id, topic_key, title, summary, status, \
                covered_from_event_id, covered_to_event_id, created_at_epoch \
         FROM topic_segments \
         WHERE project = ?1 AND topic_key = ?2 \
         ORDER BY covered_from_event_id ASC, id ASC",
    )?;
    let rows = stmt
        .query_map(params![project, topic_key], |row| {
            Ok(TopicSegmentRow {
                id: row.get(0)?,
                session_row_id: row.get(1)?,
                topic_key: row.get(2)?,
                title: row.get(3)?,
                summary: row.get(4)?,
                status: row.get(5)?,
                covered_from_event_id: row.get(6)?,
                covered_to_event_id: row.get(7)?,
                created_at_epoch: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::migrate::run_migrations(&conn).expect("migrations");
        // This unit test isolates topic_segments CRUD + overlap behavior.
        // FK integrity (host/project/session parents) is guaranteed by the real
        // capture pipeline, so we disable FKs here instead of materializing the
        // hosts/projects/sessions parent chain.
        conn.execute_batch("PRAGMA foreign_keys=OFF;")
            .expect("disable foreign keys");
        conn
    }

    fn seg<'a>(
        idx: i64,
        topic_key: &'a str,
        from: i64,
        to: i64,
        evidence: &'a str,
    ) -> TopicSegmentInput<'a> {
        TopicSegmentInput {
            host_id: 1,
            project_id: 1,
            session_row_id: 7,
            project: "/tmp/remem",
            topic_key,
            title: "title",
            summary: "summary",
            status: "resolved",
            segment_index: idx,
            covered_from_event_id: from,
            covered_to_event_id: to,
            evidence_event_ids: evidence,
            files: None,
            confidence: 0.75,
        }
    }

    #[test]
    fn insert_then_exists() -> Result<()> {
        let conn = conn();
        assert!(!topic_segment_exists(&conn, 7, "fts5-tokenizer")?);
        let id = insert_topic_segment(&conn, &seg(0, "fts5-tokenizer", 100, 110, "[100,110]"))?;
        assert!(id > 0);
        assert!(topic_segment_exists(&conn, 7, "fts5-tokenizer")?);
        assert!(!topic_segment_exists(&conn, 7, "other-topic")?);
        assert!(!topic_segment_exists(&conn, 99, "fts5-tokenizer")?);
        Ok(())
    }

    /// Phase 0 finding: interleaved tasks make segment ranges overlap/nest.
    /// The table must accept overlapping ranges across different topics.
    #[test]
    fn overlapping_segments_coexist() -> Result<()> {
        let conn = conn();
        insert_topic_segment(&conn, &seg(0, "anti-bot-research", 3056, 3466, "[3056,3466]"))?;
        insert_topic_segment(&conn, &seg(1, "kexue-scraping", 3057, 3331, "[3057,3331]"))?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM topic_segments WHERE session_row_id = 7",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 2, "overlapping segments must coexist");
        Ok(())
    }

    /// Trace Weaver: same topic_key across sessions, inserted out of order,
    /// is returned time-ordered; other topics are excluded.
    #[test]
    fn load_trace_orders_segments_across_sessions() -> Result<()> {
        let conn = conn();
        let mk = |session: i64, from: i64, to: i64| TopicSegmentInput {
            host_id: 1,
            project_id: 1,
            session_row_id: session,
            project: "/tmp/remem",
            topic_key: "fts5-tokenizer",
            title: "t",
            summary: "s",
            status: "open",
            segment_index: 0,
            covered_from_event_id: from,
            covered_to_event_id: to,
            evidence_event_ids: "[]",
            files: None,
            confidence: 0.75,
        };
        insert_topic_segment(&conn, &mk(9, 300, 310))?;
        insert_topic_segment(&conn, &mk(7, 100, 110))?;
        insert_topic_segment(&conn, &mk(8, 200, 210))?;
        let mut other = mk(7, 50, 60);
        other.topic_key = "unrelated";
        insert_topic_segment(&conn, &other)?;

        let trace = load_trace_by_topic_key(&conn, "/tmp/remem", "fts5-tokenizer")?;
        let froms: Vec<i64> = trace.iter().map(|s| s.covered_from_event_id).collect();
        assert_eq!(froms, vec![100, 200, 300], "trace ordered by event id across sessions");
        assert!(trace.iter().all(|s| s.topic_key == "fts5-tokenizer"));
        Ok(())
    }
}
