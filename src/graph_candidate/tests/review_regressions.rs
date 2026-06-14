use anyhow::Result;
use rusqlite::params;

use super::super::{process_with_graph_generator, GraphCandidateResult, GRAPH_CANDIDATE_SYSTEM};
use super::{
    graph_candidate_xml, graph_test_conn, graph_test_task, graph_test_task_with_events,
    insert_graph_entity, insert_graph_memory, insert_graph_source_observation,
    insert_graph_source_observation_with_evidence,
};
use crate::db::{self, ExtractionTaskKind};

fn graph_candidate_xml_from(
    edge_type: &str,
    from_ref: &str,
    to_ref: &str,
    evidence_id: i64,
) -> String {
    format!(
        "<graph_candidate>\
            <type>edge</type>\
            <edge_type>{edge_type}</edge_type>\
            <from_ref>{from_ref}</from_ref>\
            <to_ref>{to_ref}</to_ref>\
            <evidence_event_ids>{evidence_id}</evidence_event_ids>\
            <risk_class>low</risk_class>\
            <confidence>0.91</confidence>\
            <reason>Observation explicitly links the source to the target.</reason>\
         </graph_candidate>"
    )
}

#[tokio::test]
async fn graph_candidate_auto_promotes_episode_mentions_without_memory() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-episode-mentions",
        &["The extraction Worker entity handled the graph range."],
    )?;
    insert_graph_entity(&conn, "Worker")?;
    let event_id = event_ids[0];
    insert_graph_source_observation_with_evidence(
        &conn,
        &task,
        "The extraction Worker entity handled the graph range.",
        &[event_id],
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml_from(
            "mentions",
            &format!("episode:{event_id}"),
            "entity:Worker",
            event_id,
        ))
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
    let (from_kind, from_id, edge_type): (String, i64, String) = conn.query_row(
        "SELECT from_node_kind, from_node_id, edge_type FROM graph_edges",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(from_kind, "episode");
    assert_eq!(from_id, event_id);
    assert_eq!(edge_type, "mentions");
    Ok(())
}

#[tokio::test]
async fn invalid_auto_endpoint_does_not_rollback_batch() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-invalid-endpoint")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_entity(&conn, "Worker")?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Memory 1 mentions Worker and src/worker.rs mentions Worker.",
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "{}{}",
            graph_candidate_xml("mentions", "entity:Worker", event_id),
            graph_candidate_xml_from("mentions", "file:src/worker.rs", "entity:Worker", event_id)
        ))
    })
    .await?;

    assert_eq!(
        result,
        GraphCandidateResult::Written {
            candidates: 2,
            promoted: 1,
            pending_review: 1
        }
    );
    let statuses = conn
        .prepare("SELECT review_status FROM graph_candidates ORDER BY id ASC")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(statuses, vec!["auto_promoted", "pending_review"]);
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(edge_count, 1);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_does_not_wait_on_memory_task_with_cursor_past_graph_range() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-memory-task-cursor-past-range")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_entity(&conn, "Worker")?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 mentions the Worker entity.")?;
    let memory_task_id = db::enqueue_followup_extraction_task(
        &conn,
        &task,
        ExtractionTaskKind::MemoryCandidate,
        event_id + 10,
    )?;
    conn.execute(
        "UPDATE extraction_tasks
         SET status = 'pending',
             cursor_event_id = ?1,
             high_watermark_event_id = ?2
         WHERE id = ?3",
        params![event_id, event_id + 10, memory_task_id],
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
    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(review_status, "auto_promoted");
    Ok(())
}

#[tokio::test]
async fn graph_candidate_rejects_unpromotable_candidate_type() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-entity-alias")?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 uses Worker as an alias.")?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "<graph_candidate>\
				<type>entity_alias</type>\
				<edge_type>alias_of</edge_type>\
				<from_ref>entity:Worker</from_ref>\
				<to_ref>entity:Extraction Worker</to_ref>\
				<evidence_event_ids>{event_id}</evidence_event_ids>\
				<risk_class>low</risk_class>\
				<confidence>0.91</confidence>\
				<reason>Alias relation is not promotable yet.</reason>\
			 </graph_candidate>"
        ))
    })
    .await;
    let err = match result {
        Ok(_) => panic!("unsupported graph candidate type should fail closed"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("invalid type 'entity_alias'"));

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[test]
fn graph_candidate_system_prompt_only_requests_promotable_edge_candidates() {
    assert!(GRAPH_CANDIDATE_SYSTEM.contains("Use only edge candidates"));
    assert!(GRAPH_CANDIDATE_SYSTEM.contains("For conflicts, use only memory:<id> endpoints"));
    assert!(!GRAPH_CANDIDATE_SYSTEM.contains("entity_alias"));
    assert!(!GRAPH_CANDIDATE_SYSTEM.contains("state_relation"));
    assert!(!GRAPH_CANDIDATE_SYSTEM.contains("type=claim"));
}
