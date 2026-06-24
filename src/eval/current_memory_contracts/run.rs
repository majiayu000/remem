use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::fixture::{
    insert_current_state_commit, insert_current_state_memory_at, insert_fact, insert_state_key,
    link_current_state_commit, seed_prompt_memory, set_current_memory, set_memory_files_raw,
    set_memory_source, setup_conn, ABSTAIN_PROJECT, ABSTAIN_SESSION, HOST, PROJECT, PROMPT_PROJECT,
    PROMPT_SESSION,
};
use super::types::{
    CurrentMemoryContractCaseReport, CurrentMemoryContractEvalMetadata,
    CurrentMemoryContractEvalReport, CurrentMemoryContractMetricSummary,
    CurrentMemoryContractRateMetric, CurrentStateContractMetrics, InjectionAuditContractMetrics,
    StalenessContractMetrics, TemporalContractMetrics, UsageContractMetrics,
};

const CORPUS_NAME: &str = "builtin-current-memory-contracts-v1";

pub fn run_current_memory_contracts_eval() -> Result<CurrentMemoryContractEvalReport> {
    let conn = setup_conn().context("setup current-memory-contract eval database")?;
    seed_current_state_fixture(&conn).context("seed current-state contract fixture")?;

    let mut cases = Vec::new();
    evaluate_current_state_statuses(&conn, &mut cases)
        .context("evaluate current-state statuses")?;
    evaluate_temporal_facts(&conn, &mut cases).context("evaluate temporal fact contracts")?;
    evaluate_staleness_labels(&conn, &mut cases).context("evaluate staleness labels")?;
    evaluate_prompt_audit_and_usage(&conn, &mut cases)
        .context("evaluate prompt audit and usage contracts")?;

    let metrics = summarize_contract_metrics(&cases);
    let failing_examples = cases
        .iter()
        .filter(|case| !case.pass)
        .map(|case| {
            format!(
                "{}.{} expected {} but got {}",
                case.category, case.id, case.expected, case.actual
            )
        })
        .collect::<Vec<_>>();

    Ok(CurrentMemoryContractEvalReport {
        metadata: CurrentMemoryContractEvalMetadata {
            corpus: CORPUS_NAME.to_string(),
            storage: "in-memory sqlite with production migrations".to_string(),
            real_db_touched: false,
            project: PROJECT.to_string(),
            host: HOST.to_string(),
            scenarios: cases.len(),
        },
        metrics,
        cases,
        failing_examples,
    })
}

fn seed_current_state_fixture(conn: &Connection) -> Result<()> {
    insert_state_key(conn, 10, "repo", PROJECT, "deploy-current", "active", None)?;
    insert_current_state_memory_at(
        conn,
        101,
        10,
        "Current deploy target",
        "Use production for current deployments.",
        "active",
        100,
        None,
        None,
    )?;
    set_current_memory(conn, 10, 101)?;

    insert_state_key(conn, 20, "repo", PROJECT, "deploy-empty", "active", None)?;

    insert_state_key(conn, 30, "repo", PROJECT, "deploy-conflict", "active", None)?;
    insert_current_state_memory_at(
        conn,
        301,
        30,
        "Deploy target production",
        "Use production.",
        "active",
        300,
        None,
        None,
    )?;
    insert_current_state_memory_at(
        conn,
        302,
        30,
        "Deploy target staging conflict",
        "Use staging.",
        "active",
        301,
        None,
        None,
    )?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('conflicts', 302, 301, 30, 'contract conflict', 310)",
        [],
    )?;
    set_current_memory(conn, 30, 301)?;

    insert_state_key(
        conn,
        40,
        "repo",
        PROJECT,
        "deploy-ambiguous",
        "active",
        None,
    )?;
    insert_state_key(
        conn,
        41,
        "user",
        "user:default",
        "deploy-ambiguous",
        "active",
        None,
    )?;
    insert_current_state_memory_at(
        conn,
        401,
        40,
        "Repo deploy target",
        "Use production.",
        "active",
        400,
        None,
        None,
    )?;
    set_current_memory(conn, 40, 401)?;
    insert_current_state_memory_at(
        conn,
        402,
        41,
        "User deploy target",
        "Use staging.",
        "active",
        401,
        None,
        None,
    )?;
    set_current_memory(conn, 41, 402)?;

    insert_state_key(conn, 50, "repo", PROJECT, "deploy-facts", "active", None)?;
    insert_current_state_memory_at(
        conn,
        501,
        50,
        "Historical deploy target",
        "Use staging during the historical window.",
        "stale",
        100,
        Some(100),
        Some(200),
    )?;
    insert_current_state_memory_at(
        conn,
        502,
        50,
        "Current deploy target with facts",
        "Use production with current fact coverage.",
        "active",
        200,
        Some(200),
        None,
    )?;
    set_current_memory(conn, 50, 502)?;
    let now = chrono::Utc::now().timestamp();
    insert_fact(
        conn,
        1,
        502,
        "production",
        Some(now - 100),
        Some(now + 100),
        now - 90,
        "active",
        None,
    )?;
    insert_fact(
        conn,
        2,
        502,
        "expired",
        Some(now - 200),
        Some(now - 100),
        now - 180,
        "active",
        None,
    )?;
    insert_fact(
        conn,
        3,
        502,
        "invalidated",
        Some(now - 200),
        None,
        now - 180,
        "active",
        Some(now - 50),
    )?;
    insert_fact(
        conn,
        4,
        501,
        "staging",
        Some(100),
        Some(200),
        120,
        "active",
        None,
    )?;

    insert_state_key(
        conn,
        70,
        "repo",
        PROJECT,
        "staleness-conflict",
        "active",
        None,
    )?;
    insert_current_state_memory_at(
        conn,
        700,
        70,
        "Historical tracked memory",
        "Previous tracked source anchor remains auditable in history.",
        "stale",
        690,
        None,
        None,
    )?;
    insert_current_state_memory_at(
        conn,
        701,
        70,
        "Tracked current memory",
        "Tracked source anchor stays trusted.",
        "active",
        700,
        None,
        None,
    )?;
    insert_current_state_memory_at(
        conn,
        702,
        70,
        "Verify before trust memory",
        "Later source changes require verification before trust.",
        "active",
        701,
        None,
        None,
    )?;
    set_memory_source(conn, 700, "eval-history-session", &["src/history.rs"])?;
    set_memory_source(conn, 701, "eval-tracked-session", &["src/tracked.rs"])?;
    set_memory_source(conn, 702, "eval-verify-session", &["src/verify.rs"])?;
    link_current_state_commit(
        conn,
        1_000,
        "history-source",
        600,
        &["src/history.rs"],
        "eval-history-session",
    )?;
    link_current_state_commit(
        conn,
        1_001,
        "tracked-source",
        600,
        &["src/tracked.rs"],
        "eval-tracked-session",
    )?;
    link_current_state_commit(
        conn,
        1_002,
        "verify-source",
        600,
        &["src/verify.rs"],
        "eval-verify-session",
    )?;
    insert_current_state_commit(conn, 1_003, "verify-later", 650, &["src/verify.rs"])?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('supersedes', 700, 701, 70, 'tracked history replacement', 705)",
        [],
    )?;
    conn.execute(
        "INSERT INTO memory_edges
         (edge_type, from_memory_id, to_memory_id, state_key_id, reason, created_at_epoch)
         VALUES ('conflicts', 702, 701, 70, 'staleness contract conflict', 1200)",
        [],
    )?;
    set_current_memory(conn, 70, 701)?;

    insert_state_key(conn, 80, "repo", PROJECT, "staleness-error", "active", None)?;
    insert_current_state_memory_at(
        conn,
        801,
        80,
        "Malformed source memory",
        "Malformed source files should produce an error label.",
        "active",
        800,
        None,
        None,
    )?;
    set_current_memory(conn, 80, 801)?;
    set_memory_files_raw(conn, 801, "[not-json")?;

    Ok(())
}

fn evaluate_current_state_statuses(
    conn: &Connection,
    cases: &mut Vec<CurrentMemoryContractCaseReport>,
) -> Result<()> {
    let current = current_state_result(conn, "deploy-current", None)?;
    push_case(
        cases,
        "current_state",
        "current",
        "status=current and current id=101",
        format!(
            "status={} current={:?}",
            current.status,
            current.current.as_ref().map(|memory| memory.id)
        ),
        current.status == "current"
            && current
                .current
                .as_ref()
                .is_some_and(|memory| memory.id == 101),
    );

    let no_current = current_state_result(conn, "deploy-empty", None)?;
    push_case(
        cases,
        "current_state",
        "no_current",
        "status=no_current with no current answer",
        format!(
            "status={} current={:?}",
            no_current.status,
            no_current.current.as_ref().map(|memory| memory.id)
        ),
        no_current.status == "no_current" && no_current.current.is_none(),
    );

    let conflict = current_state_result(conn, "deploy-conflict", None)?;
    push_case(
        cases,
        "current_state",
        "unresolved_conflict",
        "status=unresolved_conflict with one conflict",
        format!(
            "status={} conflicts={}",
            conflict.status,
            conflict.conflicts.len()
        ),
        conflict.status == "unresolved_conflict" && conflict.conflicts.len() == 1,
    );

    let ambiguous = current_state_result(conn, "deploy-ambiguous", None)?;
    push_case(
        cases,
        "current_state",
        "ambiguous",
        "status=ambiguous with repo and user matches",
        format!(
            "status={} matches={}",
            ambiguous.status,
            ambiguous.matches.len()
        ),
        ambiguous.status == "ambiguous" && ambiguous.matches.len() == 2,
    );

    Ok(())
}

fn evaluate_temporal_facts(
    conn: &Connection,
    cases: &mut Vec<CurrentMemoryContractCaseReport>,
) -> Result<()> {
    let current = current_state_result(conn, "deploy-facts", None)?;
    let current_objects = fact_objects(&current);
    push_case(
        cases,
        "temporal",
        "invalidated_fact_exclusion",
        "current facts exclude invalidated",
        current_objects.join(","),
        !current_objects.iter().any(|object| object == "invalidated")
            && current_objects.iter().any(|object| object == "production"),
    );
    push_case(
        cases,
        "temporal",
        "expired_fact_exclusion",
        "current facts exclude expired",
        current_objects.join(","),
        !current_objects.iter().any(|object| object == "expired")
            && current_objects.iter().any(|object| object == "production"),
    );

    let as_of = current_state_result(conn, "deploy-facts", Some(150))?;
    let as_of_objects = fact_objects(&as_of);
    push_case(
        cases,
        "temporal",
        "as_of_fact_retrieval",
        "as_of=150 returns historical staging fact",
        format!(
            "status={} current={:?} facts={}",
            as_of.status,
            as_of.current.as_ref().map(|memory| memory.id),
            as_of_objects.join(",")
        ),
        as_of.status == "current"
            && as_of
                .current
                .as_ref()
                .is_some_and(|memory| memory.id == 501)
            && as_of_objects == vec!["staging".to_string()],
    );

    Ok(())
}

fn evaluate_staleness_labels(
    conn: &Connection,
    cases: &mut Vec<CurrentMemoryContractCaseReport>,
) -> Result<()> {
    let untracked = current_state_result(conn, "deploy-current", None)?;
    let untracked_label = untracked
        .current
        .as_ref()
        .map(|memory| memory.staleness.source_anchor.clone())
        .unwrap_or_else(|| "missing".to_string());
    push_case(
        cases,
        "staleness",
        "untracked",
        "source_anchor=untracked",
        untracked_label.clone(),
        untracked_label == "untracked",
    );

    let staleness = current_state_result(conn, "staleness-conflict", None)?;
    let tracked_label = staleness
        .current
        .as_ref()
        .map(|memory| memory.staleness.source_anchor.clone())
        .unwrap_or_else(|| "missing".to_string());
    push_case(
        cases,
        "staleness",
        "tracked",
        "source_anchor=tracked",
        tracked_label.clone(),
        tracked_label == "tracked",
    );
    let history = staleness.history.first();
    let history_actual = history
        .map(|memory| {
            format!(
                "id={} relation={:?} source_anchor={}",
                memory.id, memory.relation, memory.staleness.source_anchor
            )
        })
        .unwrap_or_else(|| "missing".to_string());
    push_case(
        cases,
        "staleness",
        "history_tracked",
        "history relation=supersedes and source_anchor=tracked",
        history_actual,
        history.is_some_and(|memory| {
            memory.id == 700
                && memory.relation.as_deref() == Some("supersedes")
                && memory.staleness.source_anchor == "tracked"
        }),
    );
    let verify_label = staleness
        .conflicts
        .first()
        .map(|memory| memory.staleness.source_anchor.clone())
        .unwrap_or_else(|| "missing".to_string());
    push_case(
        cases,
        "staleness",
        "verify_before_trust",
        "source_anchor=verify-before-trust",
        verify_label.clone(),
        verify_label == "verify-before-trust",
    );

    let error = current_state_result(conn, "staleness-error", None)?;
    let error_label = error
        .current
        .as_ref()
        .map(|memory| memory.staleness.source_anchor.clone())
        .unwrap_or_else(|| "missing".to_string());
    push_case(
        cases,
        "staleness",
        "error",
        "source_anchor=error",
        error_label.clone(),
        error_label == "error",
    );

    Ok(())
}

fn evaluate_prompt_audit_and_usage(
    conn: &Connection,
    cases: &mut Vec<CurrentMemoryContractCaseReport>,
) -> Result<()> {
    let memory_id = seed_prompt_memory(conn)?;

    let injected_output = crate::context::prompt_submit_additional_context(
        conn,
        PROMPT_PROJECT,
        PROMPT_PROJECT,
        PROMPT_SESSION,
        "SQLCipher storage decision",
        Some(HOST),
    )?;
    let injected_item_id = context_item_id(conn, PROMPT_SESSION, "injected", memory_id)?;
    let dropped_output = crate::context::prompt_submit_additional_context(
        conn,
        PROMPT_PROJECT,
        PROMPT_PROJECT,
        PROMPT_SESSION,
        "SQLCipher storage decision",
        Some(HOST),
    )?;
    let abstained_output = crate::context::prompt_submit_additional_context(
        conn,
        ABSTAIN_PROJECT,
        ABSTAIN_PROJECT,
        ABSTAIN_SESSION,
        "no matching memory should appear",
        Some(HOST),
    )?;

    let injected_count = context_item_count(conn, PROMPT_SESSION, "injected")?;
    push_case(
        cases,
        "injection",
        "audit_injected",
        "context injection audit has injected row",
        format!(
            "injected={injected_count} item_id={injected_item_id} output_present={}",
            injected_output.is_some()
        ),
        injected_count > 0 && injected_output.is_some(),
    );

    let dropped_count = context_item_count(conn, PROMPT_SESSION, "dropped")?;
    push_case(
        cases,
        "injection",
        "audit_dropped",
        "context injection audit has dropped row",
        format!(
            "dropped={dropped_count} output_present={}",
            dropped_output.is_some()
        ),
        dropped_count > 0 && dropped_output.is_none(),
    );

    let abstained_count = context_item_count(conn, ABSTAIN_SESSION, "abstained")?;
    push_case(
        cases,
        "injection",
        "audit_abstained",
        "context injection audit has abstained row",
        format!(
            "abstained={abstained_count} output_present={}",
            abstained_output.is_some()
        ),
        abstained_count > 0 && abstained_output.is_none(),
    );

    let message_hash = "eval-current-contract-citation";
    let citation_report = crate::memory::usage::record_stop_memory_citations(
        conn,
        HOST,
        PROMPT_PROJECT,
        PROMPT_SESSION,
        message_hash,
        &format!("Used the injected memory.\nMemory citations: memory:#{memory_id}"),
    )?;
    let citation_status: Option<String> = conn
        .query_row(
            "SELECT status
             FROM memory_citation_events
             WHERE message_hash = ?1
             LIMIT 1",
            [message_hash],
            |row| row.get(0),
        )
        .optional()?;
    push_case(
        cases,
        "usage",
        "citation_event_matched",
        "stop citation event status=matched",
        format!(
            "report matched={} inserted={} status={:?}",
            citation_report.matched_count, citation_report.inserted_count, citation_status
        ),
        citation_report.matched_count == 1
            && citation_report.inserted_count == 1
            && citation_status.as_deref() == Some("matched"),
    );

    let usage_linked = usage_event_count(conn, message_hash, memory_id, injected_item_id)?;
    push_case(
        cases,
        "usage",
        "usage_event_linked_to_injection_item",
        "usage event references injected context item",
        format!("linked_usage_events={usage_linked}"),
        usage_linked > 0,
    );

    Ok(())
}

fn current_state_result(
    conn: &Connection,
    state_key: &str,
    as_of_epoch: Option<i64>,
) -> Result<crate::memory::current_state::CurrentStateResult> {
    crate::memory::current_state::current_state(
        conn,
        &crate::memory::current_state::CurrentStateRequest {
            state_key: state_key.to_string(),
            project: Some(PROJECT.to_string()),
            memory_type: Some("decision".to_string()),
            as_of_epoch,
            include_history: true,
            ..Default::default()
        },
    )
}

fn fact_objects(result: &crate::memory::current_state::CurrentStateResult) -> Vec<String> {
    result
        .facts
        .iter()
        .map(|fact| fact.object.clone())
        .collect()
}

fn context_item_count(conn: &Connection, session_id: &str, status: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*)
         FROM context_injection_items
         WHERE session_id = ?1
           AND status = ?2",
        params![session_id, status],
        |row| row.get(0),
    )
    .context("count context injection items")
}

fn context_item_id(
    conn: &Connection,
    session_id: &str,
    status: &str,
    memory_id: i64,
) -> Result<i64> {
    conn.query_row(
        "SELECT id
         FROM context_injection_items
         WHERE session_id = ?1
           AND status = ?2
           AND memory_id = ?3
         ORDER BY id DESC
         LIMIT 1",
        params![session_id, status, memory_id],
        |row| row.get(0),
    )
    .optional()?
    .with_context(|| {
        format!(
            "load context injection item for session={session_id} status={status} memory_id={memory_id}"
        )
    })
}

fn usage_event_count(
    conn: &Connection,
    message_hash: &str,
    memory_id: i64,
    context_injection_item_id: i64,
) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*)
         FROM memory_usage_events
         WHERE message_hash = ?1
           AND memory_id = ?2
           AND context_injection_item_id = ?3",
        params![message_hash, memory_id, context_injection_item_id],
        |row| row.get(0),
    )
    .context("count linked memory usage events")
}

fn summarize_contract_metrics(
    cases: &[CurrentMemoryContractCaseReport],
) -> CurrentMemoryContractMetricSummary {
    CurrentMemoryContractMetricSummary {
        current_state: CurrentStateContractMetrics {
            current: rate(cases, "current_state", &["current"]),
            no_current: rate(cases, "current_state", &["no_current"]),
            unresolved_conflict: rate(cases, "current_state", &["unresolved_conflict"]),
            ambiguous: rate(cases, "current_state", &["ambiguous"]),
        },
        temporal: TemporalContractMetrics {
            invalidated_fact_exclusion: rate(cases, "temporal", &["invalidated_fact_exclusion"]),
            expired_fact_exclusion: rate(cases, "temporal", &["expired_fact_exclusion"]),
            as_of_fact_retrieval: rate(cases, "temporal", &["as_of_fact_retrieval"]),
        },
        staleness: StalenessContractMetrics {
            tracked: rate(cases, "staleness", &["tracked"]),
            untracked: rate(cases, "staleness", &["untracked"]),
            history_tracked: rate(cases, "staleness", &["history_tracked"]),
            verify_before_trust: rate(cases, "staleness", &["verify_before_trust"]),
            error: rate(cases, "staleness", &["error"]),
        },
        injection: InjectionAuditContractMetrics {
            audit_injected: rate(cases, "injection", &["audit_injected"]),
            audit_dropped: rate(cases, "injection", &["audit_dropped"]),
            audit_abstained: rate(cases, "injection", &["audit_abstained"]),
        },
        usage: UsageContractMetrics {
            citation_event_matched: rate(cases, "usage", &["citation_event_matched"]),
            usage_event_linked_to_injection_item: rate(
                cases,
                "usage",
                &["usage_event_linked_to_injection_item"],
            ),
        },
        all_checks_passed: cases.iter().all(|case| case.pass),
    }
}

fn rate(
    cases: &[CurrentMemoryContractCaseReport],
    category: &str,
    ids: &[&str],
) -> CurrentMemoryContractRateMetric {
    let passed = ids
        .iter()
        .filter(|id| {
            cases
                .iter()
                .any(|case| case.category == category && case.id == **id && case.pass)
        })
        .count();
    CurrentMemoryContractRateMetric::new(passed, ids.len())
}

fn push_case(
    cases: &mut Vec<CurrentMemoryContractCaseReport>,
    category: &str,
    id: &str,
    expected: impl Into<String>,
    actual: impl Into<String>,
    pass: bool,
) {
    cases.push(CurrentMemoryContractCaseReport {
        id: id.to_string(),
        category: category.to_string(),
        expected: expected.into(),
        actual: actual.into(),
        pass,
    });
}
