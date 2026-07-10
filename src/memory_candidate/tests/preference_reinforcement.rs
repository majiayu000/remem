use anyhow::{Context, Result};

use crate::memory_candidate::review::approve_candidate;

use super::{
    insert_source_observation, process_with_generator, setup_conn, setup_task,
    MemoryCandidateResult,
};

const PREFERENCE: &str = "Use bun, not npm, for package installation commands in this project.";

#[tokio::test]
async fn approved_duplicate_preferences_increment_canonical_reinforcement() -> Result<()> {
    let mut conn = setup_conn();

    for round in 1..=3 {
        let task = setup_task(&mut conn, &format!("sess-preference-reinforce-{round}"))?;
        insert_source_observation(&conn, &task, PREFERENCE)?;
        let result = process_with_generator(&mut conn, &task, |_prompt| async {
            Ok(format!(
                "<memory_candidate>\
                    <scope>project</scope>\
                    <type>preference</type>\
                    <topic_key>package-manager-choice</topic_key>\
                    <risk_class>low</risk_class>\
                    <confidence>0.95</confidence>\
                    <text>{PREFERENCE}</text>\
                 </memory_candidate>"
            ))
        })
        .await?;

        assert_eq!(
            result,
            MemoryCandidateResult::Written {
                candidates: 1,
                promoted: 0,
                pending_review: 1,
                to_event_id: task
                    .high_watermark_event_id
                    .context("preference task watermark")?,
            }
        );
        let candidate_id: i64 = conn.query_row(
            "SELECT id FROM memory_candidates ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        approve_candidate(&mut conn, candidate_id)?
            .context("approved preference should resolve to a memory")?;
    }

    let memory_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE memory_type = 'preference'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(memory_count, 1);
    let (count, machine_checkable, risk_class): (i64, i64, String) = conn.query_row(
        "SELECT reinforcement_count, machine_checkable, risk_class
         FROM memory_preference_reinforcements",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(count, 3);
    assert_eq!(machine_checkable, 1);
    assert_eq!(risk_class, "low");
    Ok(())
}
