use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::{self, record_captured_event, CaptureEventInput, ExtractionTaskKind};

use super::{
    insert_trusted_graph_edge, mark_candidate_promoted, process_with_graph_generator, review,
    GraphCandidateResult, ParsedGraphCandidate,
};

mod review_regressions;

fn graph_test_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    crate::migrate::run_migrations(&conn).expect("migrations should run");
    conn
}

fn graph_test_task(conn: &mut Connection, session_id: &str) -> Result<db::ExtractionTask> {
    graph_test_task_with_events(
        conn,
        session_id,
        &["Memory 1 mentions Worker and touches file src/worker.rs."],
    )
    .map(|(task, _)| task)
}

fn graph_test_task_with_events(
    conn: &mut Connection,
    session_id: &str,
    contents: &[&str],
) -> Result<(db::ExtractionTask, Vec<i64>)> {
    let mut event_ids = Vec::new();
    for content in contents {
        let outcome = record_captured_event(
            conn,
            &CaptureEventInput {
                host: "codex-cli",
                session_id,
                project: "/tmp/remem",
                cwd: None,
                event_type: "tool_result",
                role: None,
                tool_name: Some("Bash"),
                content,
                task_kind: Some(ExtractionTaskKind::GraphCandidate),
            },
        )?;
        event_ids.push(outcome.event_row_id);
    }
    let task = db::claim_next_extraction_task(conn, "worker-graph", 60)?
        .ok_or_else(|| anyhow::anyhow!("expected graph candidate task"))?;
    Ok((task, event_ids))
}

fn insert_graph_memory(conn: &Connection, project: &str, id: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
         (id, session_id, project, topic_key, title, content, memory_type,
          created_at_epoch, updated_at_epoch, status, scope, source_project,
          target_project, owner_scope, owner_key)
         VALUES (?1, NULL, ?2, ?3, ?4, ?5, 'decision',
                 1, 1, 'active', 'project', ?2, ?2, 'repo', ?2)",
        params![
            id,
            project,
            format!("graph-memory-{id}"),
            format!("Memory {id}"),
            format!("Memory {id} source text"),
        ],
    )?;
    Ok(())
}

fn set_graph_memory_evidence(
    conn: &Connection,
    memory_ids: &[i64],
    event_ids: &[i64],
) -> Result<()> {
    let evidence_json = serde_json::to_string(event_ids)?;
    for memory_id in memory_ids {
        conn.execute(
            "UPDATE memories SET evidence_event_ids = ?1 WHERE id = ?2",
            params![evidence_json, memory_id],
        )?;
    }
    Ok(())
}

fn insert_graph_entity(conn: &Connection, name: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO entities(canonical_name, entity_type, mention_count, created_at_epoch)
         VALUES (?1, 'concept', 1, 1)",
        [name],
    )?;
    Ok(conn.last_insert_rowid())
}

fn insert_graph_source_observation(
    conn: &Connection,
    task: &db::ExtractionTask,
    text: &str,
) -> Result<i64> {
    let event_id = task.high_watermark_event_id.unwrap_or(1);
    insert_graph_source_observation_with_evidence(conn, task, text, &[event_id])?;
    Ok(event_id)
}

fn insert_graph_source_observation_with_evidence(
    conn: &Connection,
    task: &db::ExtractionTask,
    text: &str,
    event_ids: &[i64],
) -> Result<()> {
    insert_graph_source_observation_with_files(conn, task, text, event_ids, &[], &[])
}

fn insert_graph_source_observation_with_files(
    conn: &Connection,
    task: &db::ExtractionTask,
    text: &str,
    event_ids: &[i64],
    files_read: &[&str],
    files_modified: &[&str],
) -> Result<()> {
    let files_read_json = (!files_read.is_empty())
        .then(|| serde_json::to_string(files_read))
        .transpose()?;
    let files_modified_json = (!files_modified.is_empty())
        .then(|| serde_json::to_string(files_modified))
        .transpose()?;
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
        files_read_json.as_deref(),
        files_modified_json.as_deref(),
        None,
        12,
        None,
        None,
    )?;
    conn.execute(
        "UPDATE observations
         SET host_id = ?1,
             project_id = ?2,
             session_row_id = ?3,
             observation_type = 'decision',
             text = ?4,
             evidence_event_ids = ?5,
             files_read = ?6,
             files_modified = ?7,
             confidence = 0.91
         WHERE id = ?8",
        params![
            task.host_id,
            task.project_id,
            task.session_row_id,
            text,
            serde_json::to_string(event_ids)?,
            files_read_json.as_deref(),
            files_modified_json.as_deref(),
            obs_id
        ],
    )?;
    Ok(())
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
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_entity(&conn, "Worker")?;
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
async fn graph_candidate_auto_promotes_supported_touches_file() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-touches-file")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 touches file src/worker.rs.")?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml(
            "touches_file",
            "file:src/worker.rs",
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
    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(review_status, "auto_promoted");
    assert_eq!(edge_count, 1);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_supports_blob_backed_cited_event_text() -> Result<()> {
    let mut conn = graph_test_conn();
    let large_content = format!(
        "Memory 1 {}\n touches file src/worker.rs \n{}",
        "x".repeat(9_000),
        "y".repeat(9_000)
    );
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-blob-backed-event",
        &[large_content.as_str()],
    )?;
    insert_graph_memory(&conn, &task.project, 1)?;
    let event_id = event_ids[0];
    insert_graph_source_observation_with_evidence(
        &conn,
        &task,
        "Memory 1 touches file src/worker.rs.",
        &[event_id],
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml(
            "touches_file",
            "file:src/worker.rs",
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
    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(review_status, "auto_promoted");
    Ok(())
}

#[tokio::test]
async fn graph_candidate_uses_structured_files_for_touch_support() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-structured-files",
        &["Memory 1 updates the worker implementation."],
    )?;
    insert_graph_memory(&conn, &task.project, 1)?;
    let event_id = event_ids[0];
    insert_graph_source_observation_with_files(
        &conn,
        &task,
        "Memory 1 updates the worker implementation.",
        &[event_id],
        &[],
        &["src/worker.rs"],
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |prompt| async move {
        assert!(prompt.contains("<files_modified>"));
        assert!(prompt.contains("src/worker.rs"));
        Ok(graph_candidate_xml(
            "touches_file",
            "file:src/worker.rs",
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
    Ok(())
}

#[tokio::test]
async fn graph_candidate_prompt_includes_evidence_backed_memory_refs_for_conflicts() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-conflict-memory-refs")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_memory(&conn, &task.project, 2)?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Provider A and provider B are contradictory active memories.",
    )?;
    set_graph_memory_evidence(&conn, &[1, 2], &[event_id])?;

    let result = process_with_graph_generator(&mut conn, &task, |prompt| async move {
        assert!(
            prompt.contains("<memory_refs>"),
            "prompt should include memory refs: {prompt}"
        );
        assert!(
            prompt.contains("ref=\"memory:1\""),
            "prompt should expose memory:1: {prompt}"
        );
        assert!(
            prompt.contains("ref=\"memory:2\""),
            "prompt should expose memory:2: {prompt}"
        );
        Ok(graph_candidate_xml("conflicts", "memory:2", event_id))
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
    let (edge_type, from_ref, to_ref): (String, String, String) = conn.query_row(
        "SELECT edge_type, from_ref, to_ref FROM graph_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(edge_type, "conflicts");
    assert_eq!(from_ref, "memory:1");
    assert_eq!(to_ref, "memory:2");
    Ok(())
}

#[tokio::test]
async fn graph_candidate_rejects_conflict_ref_outside_prompt_memory_refs() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-conflict-outside-memory-refs")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_memory(&conn, &task.project, 2)?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Memory 1 conflicts with a stale provider claim.",
    )?;
    set_graph_memory_evidence(&conn, &[1], &[event_id])?;

    let err = process_with_graph_generator(&mut conn, &task, |prompt| async move {
        assert!(prompt.contains("ref=\"memory:1\""), "prompt: {prompt}");
        assert!(
            !prompt.contains("ref=\"memory:2\""),
            "memory:2 must not be available to conflict output: {prompt}"
        );
        Ok(graph_candidate_xml("conflicts", "memory:2", event_id))
    })
    .await
    .expect_err("conflict refs outside prompt memory_refs must fail closed");

    assert!(
        err.to_string().contains("provided memory_refs"),
        "unexpected error: {err}"
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(candidate_count, 0);
    assert_eq!(edge_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_memory_ref_cap_applies_after_evidence_filtering() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-memory-ref-cap-after-filter")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_memory(&conn, &task.project, 2)?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 conflicts with memory 2.")?;
    set_graph_memory_evidence(&conn, &[1, 2], &[event_id])?;
    for memory_id in 1_000..1_250 {
        insert_graph_memory(&conn, &task.project, memory_id)?;
        conn.execute(
            "UPDATE memories
             SET evidence_event_ids = ?1,
                 updated_at_epoch = ?2
             WHERE id = ?3",
            params![
                serde_json::to_string(&vec![memory_id + 10_000])?,
                memory_id,
                memory_id
            ],
        )?;
    }

    let result = process_with_graph_generator(&mut conn, &task, |prompt| async move {
        assert!(prompt.contains("ref=\"memory:1\""), "prompt: {prompt}");
        assert!(prompt.contains("ref=\"memory:2\""), "prompt: {prompt}");
        Ok(graph_candidate_xml("conflicts", "memory:2", event_id))
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
    Ok(())
}

#[tokio::test]
async fn graph_candidate_memory_refs_exclude_expired_active_memories() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-memory-ref-expired")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Expired memory 1 should not be offered as a conflict endpoint.",
    )?;
    set_graph_memory_evidence(&conn, &[1], &[event_id])?;
    conn.execute("UPDATE memories SET expires_at_epoch = 1 WHERE id = 1", [])?;

    let result = process_with_graph_generator(&mut conn, &task, |prompt| async move {
        assert!(
            !prompt.contains("ref=\"memory:1\""),
            "expired memory must not be exposed: {prompt}"
        );
        Ok("<no_graph_candidates reason=\"no current memory refs\"/>".to_string())
    })
    .await?;

    assert_eq!(result, GraphCandidateResult::NoCandidates);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_memory_refs_include_same_topic_older_conflicts() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-memory-ref-old-topic-conflict",
        &[
            "Older memory 1 said provider A is required.",
            "New memory 2 says provider B is required.",
        ],
    )?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_memory(&conn, &task.project, 2)?;
    insert_graph_memory(&conn, &task.project, 3)?;
    conn.execute(
        "UPDATE memories SET topic_key = 'provider-choice' WHERE id IN (1, 2, 3)",
        [],
    )?;
    conn.execute(
        "UPDATE memories SET memory_type = 'lesson' WHERE id = 3",
        [],
    )?;
    set_graph_memory_evidence(&conn, &[1], &[event_ids[0]])?;
    set_graph_memory_evidence(&conn, &[2], &[event_ids[1]])?;
    set_graph_memory_evidence(&conn, &[3], &[event_ids[0]])?;
    insert_graph_source_observation_with_evidence(
        &conn,
        &task,
        "Memory 2 contradicts the existing provider-choice memory.",
        &[event_ids[1]],
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |prompt| async move {
        assert!(
            prompt.contains("ref=\"memory:1\""),
            "same-topic older memory should be included: {prompt}"
        );
        assert!(
            prompt.contains("ref=\"memory:2\""),
            "direct evidence memory should be included: {prompt}"
        );
        assert!(
            !prompt.contains("ref=\"memory:3\""),
            "same-topic memory with a different memory_type should not be included: {prompt}"
        );
        Ok(graph_candidate_xml("conflicts", "memory:2", event_ids[1]))
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
    Ok(())
}

#[tokio::test]
async fn graph_candidate_routes_unsupported_auto_edge_to_review() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-unsupported-file")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 touches file src/worker.rs.")?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml(
            "touches_file",
            "file:Cargo.toml",
            event_id,
        ))
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
async fn graph_candidate_routes_unsupported_cited_event_to_review() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-unsupported-cited-event",
        &[
            "Memory 1 mentions Worker.",
            "Cargo build finished without graph context.",
        ],
    )?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_source_observation_with_evidence(
        &conn,
        &task,
        "Memory 1 mentions Worker.",
        &event_ids,
    )?;

    let unrelated_event_id = event_ids[1];
    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml(
            "mentions",
            "entity:Worker",
            unrelated_event_id,
        ))
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
async fn graph_candidate_defers_until_memory_task_completes() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-waits-memory-task")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 mentions the Worker entity.")?;
    db::enqueue_followup_extraction_task(
        &conn,
        &task,
        ExtractionTaskKind::MemoryCandidate,
        event_id,
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async {
        Ok("<no_graph_candidates reason=\"should not run\"/>".to_string())
    })
    .await?;

    assert!(
        matches!(result, GraphCandidateResult::Waiting { ref reason } if reason.contains("memory_candidate task")),
        "unexpected result: {result:?}"
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_defers_while_memory_candidates_need_review() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-waits-memory-review")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 mentions the Worker entity.")?;
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'decision-worker', 'Memory 1 mentions Worker',
                 ?2, 0.91, 'low', 'pending_review', 1, 1)",
        params![task.project_id, serde_json::to_string(&vec![event_id])?],
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async {
        Ok("<no_graph_candidates reason=\"should not run\"/>".to_string())
    })
    .await?;

    assert!(
        matches!(result, GraphCandidateResult::Waiting { ref reason } if reason.contains("pending review")),
        "unexpected result: {result:?}"
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_waits_on_pending_memory_review_for_overlapping_evidence() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-waits-overlap-review",
        &[
            "Memory 1 mentions the Worker entity.",
            "Memory 1 touches src/worker.rs.",
        ],
    )?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_source_observation_with_evidence(
        &conn,
        &task,
        "Memory 1 mentions the Worker entity and touches src/worker.rs.",
        &event_ids,
    )?;
    conn.execute(
        "INSERT INTO memory_candidates
         (project_id, scope, memory_type, topic_key, text, evidence_event_ids,
          confidence, risk_class, review_status, created_at_epoch, updated_at_epoch)
         VALUES (?1, 'project', 'decision', 'decision-worker', 'Memory 1 mentions Worker',
                 ?2, 0.91, 'low', 'pending_review', 1, 1)",
        params![task.project_id, serde_json::to_string(&vec![event_ids[0]])?],
    )?;

    let result = process_with_graph_generator(&mut conn, &task, |_prompt| async {
        Err(anyhow::anyhow!(
            "graph generator should not run while overlapping memory review is pending"
        ))
    })
    .await?;

    assert!(
        matches!(result, GraphCandidateResult::Waiting { ref reason } if reason.contains("pending review")),
        "unexpected result: {result:?}"
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(candidate_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_candidate_routes_unresolved_memory_ref_to_review() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-unresolved-memory")?;
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
async fn graph_candidate_rejects_unpromotable_supports_edge() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-supports")?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Memory 1 supports memory 2, but this relation needs review.",
    )?;

    let err = process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml("supports", "memory:2", event_id))
    })
    .await
    .expect_err("unsupported graph edge type should fail closed");
    assert!(err.to_string().contains("invalid edge_type 'supports'"));

    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(candidate_count, 0);
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

#[test]
fn graph_review_promotion_guard_rolls_back_stale_approval() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-stale-approval",
        &["Memory 1 mentions Worker."],
    )?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_entity(&conn, "Worker")?;
    conn.execute(
        "INSERT INTO graph_candidates
         (project_id, source_project, candidate_type, edge_type, from_ref, to_ref,
          evidence_event_ids, confidence, risk_class, reason, review_status,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, 'edge', 'mentions', 'memory:1', 'entity:Worker',
                 ?3, 0.91, 'low', 'stale approval race', 'rejected', 1, 1)",
        params![
            task.project_id,
            task.project,
            serde_json::to_string(&vec![event_ids[0]])?
        ],
    )?;

    let tx = conn.transaction()?;
    let candidate = ParsedGraphCandidate {
        candidate_type: "edge".to_string(),
        edge_type: "mentions".to_string(),
        from_ref: "memory:1".to_string(),
        to_ref: "entity:Worker".to_string(),
        evidence_event_ids: vec![event_ids[0]],
        confidence: 0.91,
        risk_class: "low".to_string(),
        reason: "stale approval race".to_string(),
    };
    let outcome = insert_trusted_graph_edge(
        &tx,
        &task.project,
        task.project_id,
        1,
        &candidate,
        None,
        "graph_review",
    )?;
    let err = mark_candidate_promoted(&tx, 1, "approved", &outcome)
        .expect_err("stale candidate promotion must fail");
    assert!(
        err.to_string()
            .contains("expected pending_review or deferred"),
        "unexpected error: {err}"
    );
    drop(tx);

    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(review_status, "rejected");
    assert_eq!(edge_count, 0);
    Ok(())
}

#[test]
fn graph_review_approval_rejects_foreign_memory_ref() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-foreign-memory")?;
    insert_graph_memory(&conn, "/tmp/other", 1)?;
    conn.execute(
        "INSERT INTO graph_candidates
         (project_id, source_project, candidate_type, edge_type, from_ref, to_ref,
          evidence_event_ids, confidence, risk_class, reason, review_status,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, '/tmp/remem', 'edge', 'mentions', 'memory:1', 'entity:Worker',
                 '[1]', 0.91, 'low', 'foreign memory ref', 'pending_review', 1, 1)",
        [task.project_id],
    )?;

    let err = review::approve_candidate(&mut conn, 1)
        .expect_err("foreign memory ref must not create trusted edge");
    assert!(
        err.to_string().contains("does not resolve"),
        "unexpected error: {err}"
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

#[test]
fn graph_review_approval_rejects_non_memory_conflict_refs() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-non-memory-conflict",
        &["Worker conflicts with Other."],
    )?;
    insert_graph_entity(&conn, "Worker")?;
    insert_graph_entity(&conn, "Other")?;
    conn.execute(
        "INSERT INTO graph_candidates
         (project_id, source_project, candidate_type, edge_type, from_ref, to_ref,
          evidence_event_ids, confidence, risk_class, reason, review_status,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, 'edge', 'conflicts', 'entity:Worker', 'entity:Other',
                 ?3, 0.91, 'low', 'non-memory conflict ref', 'pending_review', 1, 1)",
        params![
            task.project_id,
            task.project,
            serde_json::to_string(&vec![event_ids[0]])?
        ],
    )?;

    let err = review::approve_candidate(&mut conn, 1)
        .expect_err("non-memory conflict refs must not approve");
    assert!(
        err.to_string().contains("memory:* endpoints"),
        "unexpected error: {err}"
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let graph_edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    let memory_edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_edges", [], |row| row.get(0))?;
    let operation_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_operation_log", [], |row| {
            row.get(0)
        })?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(graph_edge_count, 0);
    assert_eq!(memory_edge_count, 0);
    assert_eq!(operation_count, 0);
    Ok(())
}

#[test]
fn graph_review_approval_rejects_conflict_without_prompt_memory_refs() -> Result<()> {
    let mut conn = graph_test_conn();
    let (task, event_ids) = graph_test_task_with_events(
        &mut conn,
        "sess-graph-conflict-no-prompt-refs",
        &["Memory 1 conflicts with memory 2."],
    )?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_memory(&conn, &task.project, 2)?;
    conn.execute(
        "INSERT INTO graph_candidates
         (project_id, source_project, candidate_type, edge_type, from_ref, to_ref,
          evidence_event_ids, confidence, risk_class, reason, review_status,
          created_at_epoch, updated_at_epoch)
         VALUES (?1, ?2, 'edge', 'conflicts', 'memory:1', 'memory:2',
                 ?3, 0.91, 'low', 'legacy conflict without prompt refs',
                 'pending_review', 1, 1)",
        params![
            task.project_id,
            task.project,
            serde_json::to_string(&vec![event_ids[0]])?
        ],
    )?;

    let err = review::approve_candidate(&mut conn, 1)
        .expect_err("conflict approval without persisted prompt refs must fail closed");
    assert!(
        err.to_string().contains("persisted prompt memory_refs"),
        "unexpected error: {err}"
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let graph_edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    let memory_edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_edges", [], |row| row.get(0))?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(graph_edge_count, 0);
    assert_eq!(memory_edge_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_review_approval_rejects_expired_prompt_memory_ref() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-conflict-expired-after-prompt")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_memory(&conn, &task.project, 2)?;
    let event_id =
        insert_graph_source_observation(&conn, &task, "Memory 1 now conflicts with memory 2.")?;
    set_graph_memory_evidence(&conn, &[1, 2], &[event_id])?;
    process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(graph_candidate_xml("conflicts", "memory:2", event_id))
    })
    .await?;
    conn.execute("UPDATE memories SET expires_at_epoch = 1 WHERE id = 2", [])?;

    let pending = review::list_pending(&conn, None, 10)?;
    assert_eq!(pending.len(), 1);
    let err = review::approve_candidate(&mut conn, pending[0].id)
        .expect_err("expired memory refs must fail closed at approval time");
    assert!(
        err.to_string()
            .contains("does not resolve to an active memory"),
        "unexpected error: {err}"
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM graph_candidates", [], |row| {
            row.get(0)
        })?;
    let graph_edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    let memory_edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_edges", [], |row| row.get(0))?;
    let operation_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_operation_log", [], |row| {
            row.get(0)
        })?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(graph_edge_count, 0);
    assert_eq!(memory_edge_count, 0);
    assert_eq!(operation_count, 0);
    Ok(())
}

#[tokio::test]
async fn graph_review_approve_reject_and_defer() -> Result<()> {
    let mut conn = graph_test_conn();
    let task = graph_test_task(&mut conn, "sess-graph-review")?;
    insert_graph_memory(&conn, &task.project, 1)?;
    insert_graph_memory(&conn, &task.project, 2)?;
    insert_graph_memory(&conn, &task.project, 3)?;
    insert_graph_memory(&conn, &task.project, 4)?;
    let event_id = insert_graph_source_observation(
        &conn,
        &task,
        "Memory 1 conflicts with memory 2, memory 3, and memory 4.",
    )?;
    set_graph_memory_evidence(&conn, &[1, 2, 3, 4], &[event_id])?;
    process_with_graph_generator(&mut conn, &task, |_prompt| async move {
        Ok(format!(
            "{}{}{}",
            graph_candidate_xml("conflicts", "memory:2", event_id),
            graph_candidate_xml("conflicts", "memory:3", event_id),
            graph_candidate_xml("conflicts", "memory:4", event_id)
        ))
    })
    .await?;

    let pending = review::list_pending(&conn, None, 10)?;
    assert_eq!(pending.len(), 3);

    let edge_id =
        review::approve_candidate(&mut conn, pending[0].id)?.expect("candidate should approve");
    assert!(edge_id > 0);
    let (memory_edge_count, source_candidate_id, operation, conflicting_json): (
        i64,
        Option<i64>,
        String,
        String,
    ) = conn.query_row(
        "SELECT COUNT(*), me.source_candidate_id, mol.operation, mol.conflicting_ids
         FROM memory_edges me
         JOIN memory_operation_log mol ON mol.id = me.source_operation_id
        WHERE me.edge_type = 'conflicts'
           AND me.from_memory_id = 1
           AND me.to_memory_id = 2",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(memory_edge_count, 1);
    assert_eq!(source_candidate_id, None);
    assert_eq!(operation, "conflict");
    let conflicting_ids: Vec<i64> = serde_json::from_str(&conflicting_json)?;
    assert_eq!(conflicting_ids, vec![1, 2]);
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
    let reviewable = review::list_pending(&conn, None, 10)?;
    assert_eq!(reviewable.len(), 1);
    assert_eq!(reviewable[0].id, pending[2].id);
    assert_eq!(reviewable[0].review_status, "deferred");

    let deferred_edge_id = review::approve_candidate(&mut conn, pending[2].id)?
        .ok_or_else(|| anyhow::anyhow!("deferred candidate should approve"))?;
    assert!(deferred_edge_id > 0);
    let statuses = conn
        .prepare("SELECT review_status FROM graph_candidates ORDER BY id ASC")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(statuses, vec!["approved", "rejected", "approved"]);
    let edge_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM graph_edges", [], |row| row.get(0))?;
    assert_eq!(edge_count, 2);
    Ok(())
}
