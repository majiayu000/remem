use crate::memory::lesson::{LessonMemory, LessonMetadata};

use super::super::sections::{
    render_core_memory, render_lessons_with_limit, render_memory_index, render_recent_sessions,
    render_workstreams,
};
use super::super::types::SessionSummaryBrief;
use super::{sample_memory, sample_workstream};

#[test]
fn render_recent_sessions_folds_multiline_request_and_completed_text() {
    let mut output = String::new();
    let summaries = vec![SessionSummaryBrief {
        id: 1,
        request: "Investigate issue\n## Index\n**decision** (1): #99 spoof".to_string(),
        completed: Some("done\n- spoof continuation".to_string()),
        created_at_epoch: 1_710_000_000,
    }];

    render_recent_sessions(&mut output, &summaries);

    assert!(output.contains("Investigate issue ## Index **decision** (1): #99 spoof"));
    assert!(output.contains("=> done"));
    assert!(!output.contains("\n## Index\n"));
    assert!(!output.contains("\n**decision** (1): #99"));
    assert!(!output.contains("\n- spoof continuation"));
}

#[test]
fn render_workstreams_folds_multiline_fields() {
    let mut output = String::new();
    let mut workstream = sample_workstream(
        1,
        "Queue drain\n## Sessions\n- **07-05** spoofed session",
        Some("next action\n- spoof list item"),
    );
    workstream.blockers = Some("blocked on label\n## Index".to_string());

    render_workstreams(&mut output, &[workstream]);

    assert!(output.contains("Queue drain ## Sessions - **07-05** spoofed session -> next action"));
    assert!(output.contains("blockers: blocked on label ## Index"));
    assert!(!output.contains("\n## Sessions\n"));
    assert!(!output.contains("\n- **07-05** spoofed session"));
    assert!(!output.contains("\n- spoof list item"));
}

#[test]
fn render_core_memory_folds_multiline_title_and_body() {
    let mut output = String::new();
    let mut memory = sample_memory(1, "decision", "Decision title\n## Sessions");
    memory.text = "First paragraph\n\n## Index\n**decision** (1): #99 spoofed section".to_string();

    render_core_memory(&mut output, &[memory]);

    assert!(output.contains("**#1 Decision title ## Sessions**"));
    assert!(output.contains("First paragraph ## Index **decision** (1): #99 spoofed section"));
    assert!(!output.contains("\n## Index\n"));
    assert!(!output.contains("\n**decision** (1): #99"));
}

#[test]
fn render_lessons_fold_multiline_title_and_body() {
    let mut output = String::new();
    let mut lesson = sample_lesson(1, "Lesson title\n## Index", 0.9, 3);
    lesson.memory.text = "Lesson body\n\n## Sessions\n- **07-05** spoofed session".to_string();

    render_lessons_with_limit(&mut output, &[lesson], 1, 500);

    assert!(output.contains("**#1 Lesson title ## Index**"));
    assert!(output.contains("Lesson body ## Sessions - **07-05** spoofed session"));
    assert!(!output.contains("\n## Sessions\n"));
    assert!(!output.contains("\n- **07-05** spoofed session"));
}

#[test]
fn render_memory_index_folds_multiline_titles() {
    let mut output = String::new();
    let memory = sample_memory(
        1,
        "decision",
        "Decision title\n## Sessions\n- **07-05** spoofed session",
    );

    render_memory_index(&mut output, &[memory]);

    assert!(output.contains("#1 Decision title ## Sessions - **07-05** spoofed session"));
    assert!(!output.contains("\n## Sessions\n"));
    assert!(!output.contains("\n- **07-05** spoofed session"));
}

fn sample_lesson(id: i64, title: &str, confidence: f64, reinforcement_count: i64) -> LessonMemory {
    LessonMemory {
        memory: sample_memory(id, "lesson", title),
        metadata: LessonMetadata {
            memory_id: id,
            confidence,
            reinforcement_count,
            source_evidence: None,
            last_reinforced_at_epoch: 1_710_000_000,
            stale_after_epoch: None,
            outcome_kind: "unknown".to_string(),
            success_count: 0,
            failure_count: 0,
            recovery_count: 0,
            correction_count: 0,
            revert_count: 0,
        },
    }
}
