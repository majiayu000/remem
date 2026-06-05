use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::{self, record_captured_event, CaptureEventInput, ExtractionTaskKind};

use super::{process_with_graph_generator, review, GraphCandidateResult};

fn graph_test_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    crate::migrate::run_migrations(&conn).expect("migrations should run");
    conn
}

fn graph_test_task(conn: &mut Connection, session_id: &str) -> Result<db::ExtractionTask> {
    record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project: "/tmp/remem",
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: "worker loop touched src/worker.rs",
            task_kind: Some(ExtractionTaskKind::GraphCandidate),
        },
    )?;
    db::claim_next_extraction_task(conn, "worker-graph", 60)?
        .ok_or_else(|| anyhow::anyhow!("expected graph candidate task"))
}

fn insert_graph_source_observation(
    conn: &Connection,
    task: &db::ExtractionTask,
    text: &str,
) -> Result<i64> {
    let obs_id = db::insert_observation_with_branch(
        conn,
        "capture-graph-test",
        &task.project,
        "decision",
        Some("Graph source"),
        None,
        Some(text),
        None,
        None,
        None,
        None,
        None,
        12,
        None,
        None,
    )?;
    let event_id = task.high_watermark_event_id.unwrap_or(1);
    conn.execute(
        "UPDATE observations
         SET host_id = ?1,
             project_id = ?2,
             session_row_id = ?3,
             observation_type = 'decision',
             text = ?4,
             evidence_event_ids = ?5,
             confidence = 0.91
         WHERE id = ?6",
        params![
            task.host_id,
            task.project_id,
            task.session_row_id,
            text,
            serde_json::to_string(&vec![event_id])?,
            obs_id
        ],
    )?;
    Ok(event_id)
}

fn graph_candidate_xml(edge_type: &str, to_ref: &str, evidence_id: i64) -> String {
    format!(
        "<graph_candidate>\
            <type>edge</type>\
            <edge_type>{edge_type}</edge_type>\
            <from_ref>memory:1</from_ref>\
            <to_ref>{to_ref}</to_ref>\
            <evidence_event_ids>{evidence_id}</evidence_event_ids>\
            <risk_class>low</risk_class>\
            <confidence>0.91</confidence>\
            <reason>Observation explicitly links the memory to the target.</reason>\
         </graph_candidate>"
    )
}

#[tokio::test]
async fn graph_candidate_auto_promotes_low_risk_mentions() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-mentions")?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Memory 1 mentions the extraction Worker entity.",
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml("mentions", "entity:Worker", event_id))
    })
    .await?;

    assert_eq!(
        result,
        GraphCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0
        }
    );
    let (status, promoted_edge_id): (String, i64) = conn.query_row(
        "SELECT review_status, promoted_edge_id FROM graph_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(status, "auto_promoted");
    let (edge_type, source_candidate_id, source_operation_id): (String, i64, i64) = conn
        .query_row(
            "SELECT edge_type, source_candidate_id, source_operation_id
             FROM graph_edges WHERE id = ?1",
            params![promoted_edge_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    assert_eq!(edge_type, "mentions");
    assert_eq!(source_candidate_id, 1);
    assert!(source_operation_id > 0);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_keeps_supports_pending_review() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-supports")?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Memory 1 supports memory 2, but this relation needs review.",
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml("supports", "memory:2", event_id))
    })
    .await?;

    assert_eq!(
        result,
        GraphCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1
        }
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(edge_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_malformed_output_fails_closed() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-bad")?;
    insert_graph_source_observation(&conn, &task, "Memory 1 mentions Worker.")?;

    let err = process_with_graph_generator(&mut conn, &task, |_prompt| async {
        Ok("not xml".to_string())
    })
    .await
    .expect_err("malformed output should fail");

    assert!(err.to_string().contains("malformed graph_candidate"));
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_review_approve_reject_and_defer() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-review")?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Memory 1 supports memory 2 and mentions Worker.",
    )?;
    process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "{}{}{}",
            graph_candidate_xml("supports", "memory:2", event_id),
            graph_candidate_xml("refutes", "memory:3", event_id),
            graph_candidate_xml("conflicts", "memory:4", event_id)
        ))
    })
    .await?;

    let pending = review::list_pending(&conn, None, 10)?;
    assert_eq!(pending.len(), 3);

    let edge_id =
        review::approve_candidate(&mut conn, pending[0].id)?.expect("candidate should approve");
    assert!(edge_id > 0);
    assert!(review::reject_candidate(
        &conn,
        pending[1].id,
        "bad conflict evidence"
    )?);
    assert!(review::defer_candidate(
        &conn,
        pending[2].id,
        "needs more context"
    )?);

    let statuses = conn
        .prepare("SELECT review_status FROM graph_candidates ORDER BY id ASC")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(statuses, vec!["approved", "rejected", "deferred"]);
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(edge_count, 1);
    Ok(())
}
