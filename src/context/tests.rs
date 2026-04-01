use crate::memory::Memory;
use crate::workstream::{WorkStream, WorkStreamStatus};

use super::sections::{
    render_core_memory, render_memory_index, render_recent_sessions, render_workstreams,
};
use super::types::SessionSummaryBrief;

#[test]
fn render_recent_sessions_truncates_completed_line() {
    let mut output = String::new();
    let summaries = vec![SessionSummaryBrief {
        request: "Implement feature".to_string(),
        completed: Some(format!("{}\nignored", "x".repeat(130))),
        created_at_epoch: 1_710_000_000,
    }];

    render_recent_sessions(&mut output, &summaries);

    assert!(output.contains("Implement feature"));
    assert!(output.contains("=> "));
    assert!(output.contains("..."));
    assert!(!output.contains("ignored"));
}

#[test]
fn render_memory_index_prioritizes_known_types() {
    let mut output = String::new();
    let memories = vec![
        sample_memory(1, "custom", "Custom title"),
        sample_memory(2, "bugfix", "Fix title"),
        sample_memory(3, "decision", "Decision title"),
    ];

    render_memory_index(&mut output, &memories);

    let decision_pos = output.find("**Decisions**").unwrap();
    let bugfix_pos = output.find("**Bug Fixes**").unwrap();
    let custom_pos = output.find("**custom**").unwrap();
    assert!(decision_pos < bugfix_pos);
    assert!(bugfix_pos < custom_pos);
}

#[test]
fn render_workstreams_includes_next_action_when_present() {
    let mut output = String::new();
    let workstreams = vec![WorkStream {
        id: 7,
        project: "demo/project".to_string(),
        title: "Refactor context".to_string(),
        description: None,
        status: WorkStreamStatus::Active,
        progress: None,
        next_action: Some("split renderers".to_string()),
        blockers: None,
        created_at_epoch: 0,
        updated_at_epoch: 0,
        completed_at_epoch: None,
    }];

    render_workstreams(&mut output, &workstreams);

    assert!(output.contains("#7 [active] Refactor context -> split renderers"));
}

#[test]
fn render_core_memory_prioritizes_higher_score_memories() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "discovery", "Lower score", now),
        sample_memory_with_epoch(2, "decision", "Higher score", now),
    ];

    render_core_memory(&mut output, &memories);

    let high_pos = output.find("**#2 Higher score**").unwrap();
    let low_pos = output.find("**#1 Lower score**").unwrap();
    assert!(high_pos < low_pos);
}

fn sample_memory(id: i64, memory_type: &str, title: &str) -> Memory {
    sample_memory_with_epoch(id, memory_type, title, 1_710_000_000)
}

fn sample_memory_with_epoch(
    id: i64,
    memory_type: &str,
    title: &str,
    updated_at_epoch: i64,
) -> Memory {
    Memory {
        id,
        session_id: None,
        project: "demo/project".to_string(),
        topic_key: None,
        title: title.to_string(),
        text: "Body".to_string(),
        memory_type: memory_type.to_string(),
        files: None,
        created_at_epoch: updated_at_epoch,
        updated_at_epoch,
        status: "active".to_string(),
        branch: None,
        scope: "project".to_string(),
    }
}
