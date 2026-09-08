use crate::workstream::{WorkStream, WorkStreamStatus};

use super::super::sections::{render_workstreams, render_workstreams_with_limits};
use super::sample_workstream;

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
        mmdd: None,
        session_intent: None,
        session_topic: None,
        display_label: None,
        session_intent_source: None,
    }];

    render_workstreams(&mut output, &workstreams);

    assert!(output.contains("#7 [active] Refactor context -> split renderers"));
}

#[test]
fn render_workstreams_includes_blockers_when_present() {
    let mut output = String::new();
    let mut workstream = sample_workstream(7, "Refactor context", Some("split renderers"));
    workstream.blockers = Some("waiting for review".to_string());

    render_workstreams(&mut output, &[workstream]);

    assert!(output.contains("blockers: waiting for review"));
}

#[test]
fn render_workstreams_respects_item_and_char_limits() {
    let mut output = String::new();
    let workstreams = vec![
        sample_workstream(1, "First stream", Some("ship the first fix")),
        sample_workstream(2, "Second stream", Some("ship the second fix")),
        sample_workstream(3, "Third stream", Some("ship the third fix")),
    ];

    render_workstreams_with_limits(&mut output, &workstreams, 2, 200);

    assert!(output.contains("#1 [active] First stream"));
    assert!(output.contains("#2 [active] Second stream"));
    assert!(!output.contains("#3 [active] Third stream"));
    assert!(output.chars().count() <= 200);
}

#[test]
fn render_workstreams_stops_at_char_limit() {
    let mut output = String::new();
    let workstreams = vec![
        sample_workstream(1, "First", Some("fix")),
        sample_workstream(2, "Second", Some("fix")),
    ];

    render_workstreams_with_limits(&mut output, &workstreams, 10, 48);

    assert!(output.contains("#1 [active] First"));
    assert!(!output.contains("#2 [active] Second"));
    assert!(output.chars().count() <= 48);
}
