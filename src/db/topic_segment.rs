use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

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

#[derive(Debug, Clone, Serialize)]
pub struct TopicTraceEntry {
    pub id: i64,
    pub topic_key: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub segment_index: i64,
    pub covered_from_event_id: i64,
    pub covered_to_event_id: i64,
    pub evidence_event_ids: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    pub created_at_epoch: i64,
    pub updated_at_epoch: i64,
}

pub fn insert_topic_segment(conn: &Connection, seg: &TopicSegmentInput<'_>) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO topic_segments
         (host_id, project_id, session_row_id, project, topic_key, title, summary,
          status, segment_index, covered_from_event_id, covered_to_event_id,
          evidence_event_ids, files, confidence, created_at_epoch, updated_at_epoch)
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

pub fn topic_segment_exists(
    conn: &Connection,
    session_row_id: i64,
    topic_key: &str,
) -> Result<bool> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT id FROM topic_segments
             WHERE session_row_id = ?1 AND topic_key = ?2 LIMIT 1",
            params![session_row_id, topic_key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub fn load_trace_by_topic_key(
    conn: &Connection,
    project: &str,
    topic_key: &str,
    limit: i64,
) -> Result<Vec<TopicTraceEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, topic_key, title, summary, status, segment_index,
                covered_from_event_id, covered_to_event_id, evidence_event_ids,
                files, created_at_epoch, updated_at_epoch
         FROM topic_segments
         WHERE project = ?1 AND topic_key = ?2
         ORDER BY covered_from_event_id ASC, segment_index ASC, id ASC
         LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![project, topic_key, limit.max(1)], |row| {
        let evidence_json: String = row.get(8)?;
        let files_json: Option<String> = row.get(9)?;
        Ok(TopicTraceRaw {
            id: row.get(0)?,
            topic_key: row.get(1)?,
            title: row.get(2)?,
            summary: row.get(3)?,
            status: row.get(4)?,
            segment_index: row.get(5)?,
            covered_from_event_id: row.get(6)?,
            covered_to_event_id: row.get(7)?,
            evidence_json,
            files_json,
            created_at_epoch: row.get(10)?,
            updated_at_epoch: row.get(11)?,
        })
    })?;

    let mut trace = Vec::new();
    for row in rows {
        trace.push(row?.try_into_entry()?);
    }
    Ok(trace)
}

struct TopicTraceRaw {
    id: i64,
    topic_key: String,
    title: String,
    summary: String,
    status: String,
    segment_index: i64,
    covered_from_event_id: i64,
    covered_to_event_id: i64,
    evidence_json: String,
    files_json: Option<String>,
    created_at_epoch: i64,
    updated_at_epoch: i64,
}

impl TopicTraceRaw {
    fn try_into_entry(self) -> Result<TopicTraceEntry> {
        let evidence_event_ids = serde_json::from_str::<Vec<i64>>(&self.evidence_json)
            .with_context(|| format!("parse topic_segments evidence ids for id={}", self.id))?;
        let files = match self.files_json {
            Some(raw) => Some(
                serde_json::from_str::<Vec<String>>(&raw)
                    .with_context(|| format!("parse topic_segments files for id={}", self.id))?,
            ),
            None => None,
        };
        Ok(TopicTraceEntry {
            id: self.id,
            topic_key: self.topic_key,
            title: self.title,
            summary: self.summary,
            status: self.status,
            segment_index: self.segment_index,
            covered_from_event_id: self.covered_from_event_id,
            covered_to_event_id: self.covered_to_event_id,
            evidence_event_ids,
            files,
            created_at_epoch: self.created_at_epoch,
            updated_at_epoch: self.updated_at_epoch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db");
        crate::migrate::run_migrations(&conn).expect("migrations");
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

    #[test]
    fn overlapping_segments_coexist_and_trace_orders_by_range() -> Result<()> {
        let conn = conn();
        insert_topic_segment(
            &conn,
            &seg(1, "anti-bot-research", 3056, 3466, "[3056,3466]"),
        )?;
        insert_topic_segment(&conn, &seg(0, "anti-bot-research", 100, 120, "[100,120]"))?;
        insert_topic_segment(&conn, &seg(2, "kexue-scraping", 3057, 3331, "[3057,3331]"))?;

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM topic_segments WHERE session_row_id = 7",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(count, 3, "overlapping segments must coexist");

        let trace = load_trace_by_topic_key(&conn, "/tmp/remem", "anti-bot-research", 10)?;
        assert_eq!(trace.len(), 2);
        assert_eq!(trace[0].covered_from_event_id, 100);
        assert_eq!(trace[1].covered_from_event_id, 3056);
        assert_eq!(trace[1].evidence_event_ids, vec![3056, 3466]);
        Ok(())
    }
}
