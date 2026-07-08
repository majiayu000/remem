use super::super::health_action::{queue_actions, render_action_block};

#[test]
fn queue_actions_are_empty_when_runtime_is_clear() {
    let actions = queue_actions(0, 0, 0, 0, 0, 0);
    assert!(actions.is_empty());
    assert!(render_action_block(&actions).is_empty());
}

#[test]
fn queue_actions_render_copy_paste_commands() {
    let actions = queue_actions(43, 1, 5, 2, 3, 4);
    let text = render_action_block(&actions);

    assert!(text.contains("Needs attention:"));
    assert!(text.contains("43 failed pending observations"));
    assert!(text.contains("inspect: remem pending list-failed --limit 20"));
    assert!(text.contains("preview migration prep: remem pending retry-failed --dry-run"));
    assert!(text.contains("apply migration prep: remem pending retry-failed"));
    assert!(text.contains("preview replay: remem pending migrate-legacy --dry-run"));
    assert!(text.contains("apply replay: remem pending migrate-legacy"));
    assert!(text.contains("1 expired processing pending observation"));
    assert!(text.contains("5 expired processing extraction tasks"));
    assert!(text.contains("2 failed jobs"));
    assert!(text.contains("3 stuck jobs"));
    assert!(text.contains("4 failed extraction tasks"));
    assert!(text.contains("inspect counts: remem status --json"));
    assert!(text.contains("recover: remem worker --once"));
}

#[test]
fn expired_processing_pending_rows_render_replay_commands_without_failed_rows() {
    let actions = queue_actions(0, 2, 0, 0, 0, 0);
    let text = render_action_block(&actions);

    assert!(text.contains("2 expired processing pending observations"));
    assert!(text.contains("inspect counts: remem status --json"));
    assert!(text.contains("preview replay: remem pending migrate-legacy --dry-run"));
    assert!(text.contains("apply replay: remem pending migrate-legacy"));
    assert!(!text.contains("retry-failed"));
}
