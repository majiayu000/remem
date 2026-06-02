use anyhow::Result;
use rusqlite::{params, Connection};

use crate::db::{self, record_captured_event, CaptureEventInput, ExtractionTaskKind};

use super::{process_with_generator, MemoryCandidateResult};

mod ttl;

pub(super) fn setup_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory db should open");
    crate::migrate::run_migrations(&conn).expect("migrations should run");
    conn
}

pub(super) fn setup_task(conn: &mut Connection, session_id: &str) -> Result<db::ExtractionTask> {
    setup_task_with_project(conn, session_id, "/tmp/remem")
}

fn setup_task_with_project(
    conn: &mut Connection,
    session_id: &str,
    project: &str,
) -> Result<db::ExtractionTask> {
    record_captured_event(
        conn,
        &CaptureEventInput {
            host: "codex-cli",
            session_id,
            project,
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: "cargo test passed",
            task_kind: Some(ExtractionTaskKind::MemoryCandidate),
        },
    )?;
    db::claim_next_extraction_task(conn, "worker-a", 60)?
        .ok_or_else(|| anyhow::anyhow!("expected memory candidate task"))
}

fn insert_source_observation(
    conn: &Connection,
    task: &db::ExtractionTask,
    text: &str,
) -> Result<()> {
    insert_source_observation_with_confidence(conn, task, text, 0.91)
}

fn insert_source_observation_with_confidence(
    conn: &Connection,
    task: &db::ExtractionTask,
    text: &str,
    confidence: f64,
) -> Result<()> {
    let obs_id = db::insert_observation_with_branch(
        conn,
        "capture-observation-test",
        &task.project,
        "decision",
        Some("Worker loop decision"),
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
             confidence = ?6
         WHERE id = ?7",
        params![
            task.host_id,
            task.project_id,
            task.session_row_id,
            text,
            serde_json::to_string(&vec![event_id])?,
            confidence,
            obs_id
        ],
    )?;
    Ok(())
}

#[tokio::test]
async fn memory_candidate_auto_promotes_default_confidence_observation() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-default-confidence")?;
    insert_source_observation_with_confidence(
        &conn,
        &task,
        "Use the worker loop to process extraction tasks after observation extraction.",
        0.75,
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(low_risk_candidate_xml())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0
        }
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    assert_eq!(review_status, "auto_promoted");
    Ok(())
}

fn low_risk_candidate_xml() -> String {
    "<memory_candidate>\
        <scope>project</scope>\
        <type>decision</type>\
        <topic_key>decision-worker-loop</topic_key>\
        <risk_class>low</risk_class>\
        <confidence>0.92</confidence>\
        <text>Use the worker loop to process extraction tasks after observation extraction.</text>\
     </memory_candidate>"
        .to_string()
}

#[tokio::test]
async fn memory_candidate_auto_promotes_low_risk_project_candidate() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-auto")?;
    insert_source_observation(
        &conn,
        &task,
        "Use the worker loop to process extraction tasks after observation extraction.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(low_risk_candidate_xml())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0
        }
    );
    let (
        candidate_id,
        review_status,
        owner_scope,
        owner_key,
        target_project,
        context_class,
        candidate_state_key,
    ): (i64, String, String, String, String, String, String) = conn.query_row(
        "SELECT id, review_status, owner_scope, owner_key, target_project, context_class, state_key
         FROM memory_candidates",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        },
    )?;
    assert_eq!(review_status, "auto_promoted");
    assert_eq!(owner_scope, "repo");
    assert_eq!(owner_key, "/tmp/remem");
    assert_eq!(target_project, "/tmp/remem");
    assert_eq!(context_class, "startup_core");
    assert_eq!(candidate_state_key, "decision-worker-loop");
    let (
        memory_id,
        memory_type,
        topic_key,
        evidence,
        source_candidate_id,
        confidence,
        memory_owner_scope,
        memory_owner_key,
        memory_state_key,
        current_memory_id,
    ): (
        i64,
        String,
        String,
        String,
        i64,
        f64,
        String,
        String,
        String,
        i64,
    ) = conn.query_row(
        "SELECT m.id, m.memory_type, m.topic_key, m.evidence_event_ids, m.source_candidate_id,
                m.confidence, m.owner_scope, m.owner_key, sk.state_key, sk.current_memory_id
         FROM memories m
         JOIN memory_state_keys sk ON sk.id = m.state_key_id
         WHERE m.source_candidate_id = ?1",
        params![candidate_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
            ))
        },
    )?;
    assert_eq!(memory_type, "decision");
    assert_eq!(topic_key, "decision-worker-loop");
    assert_eq!(source_candidate_id, candidate_id);
    assert!(evidence.contains('1'));
    assert_eq!(confidence, 0.92);
    assert_eq!(memory_owner_scope, "repo");
    assert_eq!(memory_owner_key, "/tmp/remem");
    assert_eq!(memory_state_key, "decision-worker-loop");
    assert_eq!(current_memory_id, memory_id);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_routes_stash_repo_fact_to_repo_owner() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task_with_project(&mut conn, "sess-candidate-stash", "/tmp/stash")?;
    let text = "Stash keeps sidebar drag and drop ordering in the project UI.";
    insert_source_observation(&conn, &task, text)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>stash-sidebar-dnd</topic_key>\
                <risk_class>low</risk_class>\
                <confidence>0.92</confidence>\
                <text>{text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0
        }
    );
    let (project, owner_scope, owner_key): (String, String, String) = conn.query_row(
        "SELECT project, owner_scope, owner_key FROM memories",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(project, "/tmp/stash");
    assert_eq!(owner_scope, "repo");
    assert_eq!(owner_key, "/tmp/stash");
    Ok(())
}

#[tokio::test]
async fn memory_candidate_routes_codex_sandbox_fact_to_tool_review() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task_with_project(&mut conn, "sess-candidate-codex", "/tmp/stash")?;
    let text = "Codex CLI sandbox approvals must use workspace-write for local repo edits.";
    insert_source_observation(&conn, &task, text)?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>codex-sandbox-approval</topic_key>\
                <risk_class>low</risk_class>\
                <confidence>0.94</confidence>\
                <text>{text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1
        }
    );
    let (review_status, owner_scope, owner_key, context_class): (String, String, String, String) =
        conn.query_row(
            "SELECT review_status, owner_scope, owner_key, context_class FROM memory_candidates",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(owner_scope, "tool");
    assert_eq!(owner_key, "codex-cli");
    assert_eq!(context_class, "search_only");
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_routes_domain_and_user_facts_without_auto_promote() -> Result<()> {
    let mut conn = setup_conn();
    let grok_task = setup_task_with_project(&mut conn, "sess-candidate-grok", "/tmp/stash")?;
    let grok_text = "Grok API streaming responses require xAI-compatible retry handling.";
    insert_source_observation(&conn, &grok_task, grok_text)?;
    process_with_generator(&mut conn, &grok_task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>grok-api-streaming</topic_key>\
                <risk_class>low</risk_class>\
                <confidence>0.93</confidence>\
                <text>{grok_text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    let macos_task = setup_task_with_project(&mut conn, "sess-candidate-macos", "/tmp/stash")?;
    let macos_text = "Warp on macOS may need TCC reset when the app bundle identity changes.";
    insert_source_observation(&conn, &macos_task, macos_text)?;
    process_with_generator(&mut conn, &macos_task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>warp-macos-tcc</topic_key>\
                <risk_class>low</risk_class>\
                <confidence>0.93</confidence>\
                <text>{macos_text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    let pref_task = setup_task_with_project(&mut conn, "sess-candidate-pref", "/tmp/stash")?;
    let pref_text = "User prefers concise Chinese progress updates during long-running work.";
    insert_source_observation(&conn, &pref_task, pref_text)?;
    process_with_generator(&mut conn, &pref_task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>global</scope>\
                <type>preference</type>\
                <topic_key>concise-chinese-progress</topic_key>\
                <risk_class>medium</risk_class>\
                <confidence>0.91</confidence>\
                <text>{pref_text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    let routes = conn
        .prepare(
            "SELECT topic_key, owner_scope, owner_key, review_status
             FROM memory_candidates
             ORDER BY id ASC",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(
        routes,
        vec![
            (
                "grok-api-streaming".to_string(),
                "domain".to_string(),
                "grok-api".to_string(),
                "pending_review".to_string()
            ),
            (
                "warp-macos-tcc".to_string(),
                "domain".to_string(),
                "macos".to_string(),
                "pending_review".to_string()
            ),
            (
                "concise-chinese-progress".to_string(),
                "user".to_string(),
                "user:default".to_string(),
                "pending_review".to_string()
            )
        ]
    );
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_existing_same_topic_same_text_becomes_noop() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-noop")?;
    let text = "Use the worker loop to process extraction tasks after observation extraction.";
    insert_source_observation(&conn, &task, text)?;
    crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("decision-worker-loop"),
        "Existing worker loop",
        text,
        "decision",
        None,
        None,
        "project",
        None,
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(low_risk_candidate_xml())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 0
        }
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(review_status, "noop");
    assert_eq!(memory_count, 1);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_newer_same_topic_supersedes_old_memory() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-update")?;
    let text =
        "Use the async worker loop to process extraction tasks after observation extraction.";
    insert_source_observation(&conn, &task, text)?;
    let old_id = crate::memory::insert_memory_full(
        &conn,
        None,
        "/tmp/remem",
        Some("decision-worker-loop"),
        "Old worker loop",
        "Use the synchronous worker loop to process extraction tasks after observation extraction.",
        "decision",
        None,
        None,
        "project",
        None,
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(format!(
            "<memory_candidate>\
                <scope>project</scope>\
                <type>decision</type>\
                <topic_key>decision-worker-loop</topic_key>\
                <risk_class>low</risk_class>\
                <confidence>0.92</confidence>\
                <text>{text}</text>\
             </memory_candidate>"
        ))
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 1,
            pending_review: 0
        }
    );
    let old_status: String = conn.query_row(
        "SELECT status FROM memories WHERE id = ?1",
        params![old_id],
        |row| row.get(0),
    )?;
    let active_text: String = conn.query_row(
        "SELECT content FROM memories
         WHERE topic_key = 'decision-worker-loop' AND status = 'active'",
        [],
        |row| row.get(0),
    )?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(old_status, "stale");
    assert_eq!(active_text, text);
    assert_eq!(memory_count, 2);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_auto_promote_requires_route_and_evidence() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-no-evidence")?;
    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(low_risk_candidate_xml())
    })
    .await?;

    assert_eq!(result, MemoryCandidateResult::EmptyRange);
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_keeps_self_classified_unsupported_candidate_pending() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-unsupported")?;
    insert_source_observation(
        &conn,
        &task,
        "Use deterministic review gates for candidates.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>decision</type><topic_key>unsupported-auto</topic_key><risk_class>low</risk_class><confidence>0.99</confidence><text>The production deploy succeeded and should be recorded.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1
        }
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_accepts_procedure_but_keeps_it_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-procedure")?;
    insert_source_observation(
        &conn,
        &task,
        "Run cargo test after memory type registry changes.",
    )?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>project</scope><type>procedure</type><topic_key>procedure-cargo-test</topic_key><risk_class>low</risk_class><confidence>0.95</confidence><text>Run cargo test after memory type registry changes.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1
        }
    );
    let (memory_type, review_status): (String, String) = conn.query_row(
        "SELECT memory_type, review_status FROM memory_candidates",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(memory_type, "procedure");
    assert_eq!(review_status, "pending_review");
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_leaves_high_risk_candidate_pending_review() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-pending")?;
    insert_source_observation(&conn, &task, "User prefers global editor behavior.")?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<memory_candidate><scope>global</scope><type>preference</type><topic_key>global-editor</topic_key><risk_class>high</risk_class><confidence>0.95</confidence><text>User prefers global editor behavior.</text></memory_candidate>".to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Written {
            candidates: 1,
            promoted: 0,
            pending_review: 1
        }
    );
    let review_status: String =
        conn.query_row("SELECT review_status FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(review_status, "pending_review");
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_duplicate_output_is_idempotent() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-dup")?;
    insert_source_observation(
        &conn,
        &task,
        "Use the worker loop to process extraction tasks after observation extraction.",
    )?;

    process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(low_risk_candidate_xml())
    })
    .await?;
    let second = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(low_risk_candidate_xml())
    })
    .await?;

    assert_eq!(
        second,
        MemoryCandidateResult::Written {
            candidates: 0,
            promoted: 0,
            pending_review: 0
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(candidate_count, 1);
    assert_eq!(memory_count, 1);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_accepts_explicit_no_candidates() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-none")?;
    insert_source_observation(&conn, &task, "Low signal output.")?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<no_candidates reason=\"low signal\"/>".to_string())
    })
    .await?;

    assert_eq!(result, MemoryCandidateResult::NoCandidates);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_defer_output_is_explicit() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-defer")?;
    insert_source_observation(&conn, &task, "Deploy target is staging or production.")?;

    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("<defer reason=\"ambiguous conflict\"/>".to_string())
    })
    .await?;

    assert_eq!(
        result,
        MemoryCandidateResult::Deferred {
            reason: "ambiguous conflict".to_string()
        }
    );
    let candidate_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memory_candidates", [], |row| {
            row.get(0)
        })?;
    let memory_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
    assert_eq!(candidate_count, 0);
    assert_eq!(memory_count, 0);
    Ok(())
}

#[tokio::test]
async fn memory_candidate_malformed_output_fails_closed() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-bad")?;
    insert_source_observation(&conn, &task, "Important durable decision.")?;

    let err = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok("not xml".to_string())
    })
    .await
    .expect_err("malformed output should fail");

    assert!(err.to_string().contains("malformed memory_candidate"));
    Ok(())
}
