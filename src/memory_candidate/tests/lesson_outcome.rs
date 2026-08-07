//! GH-958: extraction-declared lesson outcomes reach `memory_lessons` and
//! failure trajectories become guardrail lessons instead of being dropped.

use anyhow::Result;

use super::super::parse_candidate_output;
use super::{process_with_generator, setup_conn, setup_task};
use crate::memory_candidate::tests::insert_source_observation;
use crate::memory_candidate::MemoryCandidateResult;

fn lesson_xml(outcome_field: &str) -> String {
    format!(
        "<memory_candidate>\
            <scope>project</scope>\
            <type>lesson</type>\
            <topic_key>lesson-codesign</topic_key>\
            <risk_class>low</risk_class>\
            <confidence>0.9</confidence>\
            {outcome_field}\
            <text>Replacing the binary with cp without codesign made macOS SIGKILL it; re-sign before running.</text>\
         </memory_candidate>"
    )
}

#[test]
fn parse_accepts_lesson_outcome_and_drops_invalid_values() -> Result<()> {
    let candidates = parse_candidate_output(&lesson_xml("<outcome>failure</outcome>"))?;
    assert_eq!(candidates[0].outcome.as_deref(), Some("failure"));

    let candidates = parse_candidate_output(&lesson_xml("<outcome>SUCCESS</outcome>"))?;
    assert_eq!(candidates[0].outcome.as_deref(), Some("success"));

    let candidates = parse_candidate_output(&lesson_xml(""))?;
    assert_eq!(candidates[0].outcome, None);

    let candidates = parse_candidate_output(&lesson_xml("<outcome>победа</outcome>"))?;
    assert_eq!(
        candidates[0].outcome, None,
        "unknown outcome must drop, not fail"
    );

    // Outcome on a non-lesson type is dropped.
    let decision = "<memory_candidate>\
        <scope>project</scope><type>decision</type><topic_key>d-1</topic_key>\
        <risk_class>low</risk_class><confidence>0.9</confidence>\
        <outcome>failure</outcome>\
        <text>The retry policy decision stands as recorded.</text>\
     </memory_candidate>";
    let candidates = parse_candidate_output(decision)?;
    assert_eq!(candidates[0].outcome, None);
    Ok(())
}

#[tokio::test]
async fn promoted_failure_lesson_records_outcome_kind() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-lesson-outcome")?;
    insert_source_observation(
        &conn,
        &task,
        "Replacing the binary with cp without codesign made macOS SIGKILL it; re-sign before running.",
    )?;
    let result = process_with_generator(&mut conn, &task, |_prompt| async {
        Ok(lesson_xml("<outcome>failure</outcome>"))
    })
    .await?;
    assert!(matches!(result, MemoryCandidateResult::Written { .. }));

    // Lessons route to pending_review; the outcome must survive the DB
    // round-trip and reach memory_lessons through the approval transaction.
    let candidate_id: i64 = conn.query_row("SELECT MAX(id) FROM memory_candidates", [], |row| {
        row.get(0)
    })?;
    crate::memory_candidate::review::approve_candidate(&mut conn, candidate_id)?
        .expect("lesson candidate should approve");

    let (outcome_kind, failure_count, success_count): (String, i64, i64) = conn.query_row(
        "SELECT l.outcome_kind, l.failure_count, l.success_count
         FROM memory_lessons l
         JOIN memories m ON m.id = l.memory_id
         WHERE m.topic_key = 'lesson-codesign'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(outcome_kind, "failure");
    assert_eq!(failure_count, 1);
    assert_eq!(success_count, 0);
    Ok(())
}
