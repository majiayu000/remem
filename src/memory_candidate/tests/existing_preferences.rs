use anyhow::Result;
use std::sync::{Arc, Mutex};

use super::{insert_source_observation, setup_conn, setup_task};
use crate::memory_candidate::{process_with_generator, MemoryCandidateResult};

#[tokio::test]
async fn memory_candidate_prompt_includes_existing_project_preferences() -> Result<()> {
    let mut conn = setup_conn();
    let task = setup_task(&mut conn, "sess-candidate-existing-preferences")?;
    crate::memory::insert_memory(
        &conn,
        None,
        &task.project,
        Some("concise-chinese-progress"),
        "Preference: concise Chinese progress",
        "Prefer concise Chinese progress updates during long-running work.",
        "preference",
        None,
    )?;
    crate::memory::insert_memory(
        &conn,
        None,
        "/tmp/other",
        Some("other-project-pref"),
        "Preference: other project",
        "Prefer other project release notes.",
        "preference",
        None,
    )?;
    insert_source_observation(
        &conn,
        &task,
        "User again asked for concise Chinese progress updates during long-running work.",
    )?;

    let captured_prompt = Arc::new(Mutex::new(String::new()));
    let captured_for_generator = Arc::clone(&captured_prompt);
    let result = process_with_generator(&mut conn, &task, move |prompt| {
        let captured_for_generator = Arc::clone(&captured_for_generator);
        async move {
            *captured_for_generator.lock().expect("prompt lock") = prompt;
            Ok("<no_candidates reason=\"preference already active\"/>".to_string())
        }
    })
    .await?;

    assert_eq!(result, MemoryCandidateResult::NoCandidates);
    let prompt = captured_prompt.lock().expect("prompt lock").clone();
    assert!(prompt.contains("<existing_active_preferences>"));
    assert!(prompt.contains("new evidence of the same correction"));
    assert!(prompt.contains("count an evidence-backed reinforcement"));
    assert!(prompt.contains("Do not emit unsupported restatements"));
    assert!(prompt.contains("Prefer concise Chinese progress updates"));
    assert!(!prompt.contains("Prefer other project release notes"));
    Ok(())
}
