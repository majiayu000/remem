//! GH-956: extraction-produced SPO facts reach `memory_facts` through the
//! candidate promotion transaction, and contradictions supersede instead of
//! deleting.

use anyhow::Result;

use super::super::fact_extract::{facts_from_json, facts_to_json, ParsedCandidateFact};
use super::super::parse_candidate_output;
use super::{process_with_generator, setup_conn, setup_task};
use crate::memory::facts::FactPredicate;
use crate::memory_candidate::tests::insert_source_observation;
use crate::memory_candidate::MemoryCandidateResult;

fn candidate_xml_with_facts(facts: &str) -> String {
    candidate_xml(
        "decision-worker-loop",
        "Use the worker loop to process extraction tasks after observation extraction.",
        facts,
    )
}

fn candidate_xml(topic_key: &str, text: &str, facts: &str) -> String {
    format!(
        "<memory_candidate>\
            <scope>project</scope>\
            <type>decision</type>\
            <topic_key>{topic_key}</topic_key>\
            <risk_class>low</risk_class>\
            <confidence>0.92</confidence>\
            <text>{text}</text>\
            {facts}\
         </memory_candidate>"
    )
}

#[test]
fn parse_extracts_spo_facts_and_drops_invalid_predicates() -> Result<()> {
    let output = candidate_xml_with_facts(
        "<fact subject=\"worker-loop\" predicate=\"uses_command\" object=\"cargo test\"/>\
         <fact subject=\"worker-loop\" predicate=\"made_of_cheese\" object=\"nope\"/>\
         <fact subject=\"worker-loop\" predicate=\"uses_file\" object=\"src/worker.rs\"/>",
    );
    let candidates = parse_candidate_output(&output)?;
    assert_eq!(candidates.len(), 1);
    let facts = &candidates[0].facts;
    assert_eq!(
        facts,
        &vec![
            ParsedCandidateFact {
                subject: "worker-loop".to_string(),
                predicate: FactPredicate::UsesCommand,
                object: "cargo test".to_string(),
            },
            ParsedCandidateFact {
                subject: "worker-loop".to_string(),
                predicate: FactPredicate::UsesFile,
                object: "src/worker.rs".to_string(),
            },
        ]
    );
    Ok(())
}

#[test]
fn facts_json_round_trip_preserves_triples() {
    let facts = vec![
        ParsedCandidateFact {
            subject: "auth".to_string(),
            predicate: FactPredicate::BlockedBy,
            object: "GH-42".to_string(),
        },
        ParsedCandidateFact {
            subject: "auth".to_string(),
            predicate: FactPredicate::FixedBy,
            object: "abc123".to_string(),
        },
    ];
    let encoded = facts_to_json(&facts).expect("non-empty facts encode");
    assert_eq!(facts_from_json(Some(&encoded)), facts);
    assert!(facts_to_json(&[]).is_none());
    assert!(facts_from_json(None).is_empty());
    assert!(facts_from_json(Some("not json")).is_empty());
}

fn current_object(
    conn: &rusqlite::Connection,
    project: &str,
    subject: &str,
    predicate: FactPredicate,
) -> Result<Option<String>> {
    let facts =
        crate::memory::facts::list_current_facts(conn, project, Some(subject), Some(predicate))?;
    Ok(facts.into_iter().next().map(|fact| fact.object))
}

#[tokio::test]
async fn promoted_candidate_writes_and_supersedes_spo_facts() -> Result<()> {
    let mut conn = setup_conn();
    let project = "/tmp/remem";

    let task = setup_task(&mut conn, "sess-spo-1")?;
    insert_source_observation(
        &conn,
        &task,
        "Use the worker loop to process extraction tasks after observation extraction.",
    )?;
    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(candidate_xml_with_facts(
            "<fact subject=\"worker-loop\" predicate=\"blocked_by\" object=\"GH-100\"/>",
        ))
    })
    .await?;
    assert!(matches!(result, MemoryCandidateResult::Written { .. }));
    assert_eq!(
        current_object(&conn, project, "worker-loop", FactPredicate::BlockedBy)?.as_deref(),
        Some("GH-100")
    );

    // A later candidate contradicts the blocker: the old fact must close via
    // valid_to/supersede, never disappear.
    let task = setup_task(&mut conn, "sess-spo-2")?;
    insert_source_observation(
        &conn,
        &task,
        "The worker loop blocker moved to GH-200 after triage.",
    )?;
    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(candidate_xml(
            "decision-worker-loop-blocker",
            "The worker loop blocker moved to GH-200 after triage.",
            "<fact subject=\"worker-loop\" predicate=\"blocked_by\" object=\"GH-200\"/>",
        ))
    })
    .await?;
    assert!(matches!(result, MemoryCandidateResult::Written { .. }));

    assert_eq!(
        current_object(&conn, project, "worker-loop", FactPredicate::BlockedBy)?.as_deref(),
        Some("GH-200")
    );
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts WHERE subject = 'worker-loop' AND predicate = 'blocked_by'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(total, 2, "supersede must keep history, not delete");
    let superseded: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts
         WHERE subject = 'worker-loop' AND predicate = 'blocked_by'
           AND object = 'GH-100' AND valid_to_epoch IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(superseded, 1);
    Ok(())
}

#[tokio::test]
async fn same_object_fact_is_idempotent() -> Result<()> {
    let mut conn = setup_conn();
    for session in ["sess-spo-idem-1", "sess-spo-idem-2"] {
        let task = setup_task(&mut conn, session)?;
        insert_source_observation(
            &conn,
            &task,
            "Use the worker loop to process extraction tasks after observation extraction.",
        )?;
        process_with_generator(&mut conn, &task, |_prompt| async {
            Ok(candidate_xml_with_facts(
                "<fact subject=\"worker-loop\" predicate=\"verified_by\" object=\"cargo test\"/>",
            ))
        })
        .await?;
    }
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts
         WHERE subject = 'worker-loop' AND predicate = 'verified_by'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(total, 1, "unchanged triple must not duplicate");
    Ok(())
}

#[test]
fn supersede_sequences_keep_history_reconstructible_with_zero_hard_deletes() -> Result<()> {
    let conn = setup_conn();
    let project = "/tmp/remem";
    crate::db::record_captured_event(
        &conn,
        &crate::db::CaptureEventInput {
            host: "codex-cli",
            session_id: "sess-prop",
            project,
            cwd: None,
            event_type: "tool_result",
            role: None,
            tool_name: Some("Bash"),
            content: "evidence for the property sequence",
            task_kind: None,
        },
    )?;
    let event_id: i64 =
        conn.query_row("SELECT MAX(id) FROM captured_events", [], |row| row.get(0))?;
    conn.execute(
        "INSERT INTO memories
         (project, title, content, memory_type, created_at_epoch, updated_at_epoch, status)
         VALUES (?1, 'prop memory', 'source memory for property facts', 'decision', 1000, 1000, 'active')",
        [project],
    )?;
    let memory_id = conn.last_insert_rowid();

    // Deterministic supersede sequence over one (subject, predicate) key:
    // each step advances valid_from and swaps the object.
    let objects = ["o-a", "o-b", "o-a", "o-c", "o-b", "o-b", "o-d"];
    let base_epoch = 1_600_000_000_i64;
    let mut expected_history: Vec<(i64, String)> = Vec::new();
    for (step, object) in objects.iter().enumerate() {
        let valid_from = base_epoch + (step as i64) * 3_600;
        let fact = ParsedCandidateFact {
            subject: "prop-subject".to_string(),
            predicate: FactPredicate::AffectsProject,
            object: object.to_string(),
        };
        super::super::fact_extract::write_candidate_facts(
            &conn,
            project,
            memory_id,
            std::slice::from_ref(&fact),
            &[event_id],
            valid_from,
            0.9,
        )?;
        if expected_history.last().map(|(_, object)| object.as_str()) != Some(object) {
            expected_history.push((valid_from, object.to_string()));
        }
    }

    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memory_facts WHERE subject = 'prop-subject'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        total,
        expected_history.len() as i64,
        "every distinct transition is one row; nothing is ever deleted"
    );

    // The stored rows reconstruct the exact valid-time history: rows ordered
    // by valid_from replay the transition sequence, every superseded row's
    // valid_to closes exactly where its successor opens, and only the final
    // row stays open-ended.
    let mut stmt = conn.prepare(
        "SELECT object, valid_from_epoch, valid_to_epoch
         FROM memory_facts
         WHERE subject = 'prop-subject'
         ORDER BY valid_from_epoch",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    let history = crate::db::query::collect_rows(rows)?;
    assert_eq!(history.len(), expected_history.len());
    for (index, ((object, valid_from, valid_to), (expected_from, expected_object))) in
        history.iter().zip(&expected_history).enumerate()
    {
        assert_eq!(object, expected_object);
        assert_eq!(valid_from, expected_from);
        let expected_to = expected_history.get(index + 1).map(|(from, _)| *from);
        assert_eq!(
            *valid_to, expected_to,
            "each superseded row must close exactly where its successor opens"
        );
    }
    // Current view converges on the last written object.
    assert_eq!(
        current_object(
            &conn,
            project,
            "prop-subject",
            FactPredicate::AffectsProject
        )?
        .as_deref(),
        Some(objects[objects.len() - 1])
    );
    Ok(())
}
