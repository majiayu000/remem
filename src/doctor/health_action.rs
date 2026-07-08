#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HealthAction {
    title: String,
    commands: Vec<HealthCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HealthCommand {
    label: &'static str,
    command: &'static str,
}

impl HealthAction {
    fn new(title: String) -> Self {
        Self {
            title,
            commands: Vec::new(),
        }
    }

    fn command(mut self, label: &'static str, command: &'static str) -> Self {
        self.commands.push(HealthCommand { label, command });
        self
    }
}

pub(crate) fn queue_actions(
    failed_pending_observations: i64,
    expired_processing_pending_observations: i64,
    expired_processing_extraction_tasks: i64,
    failed_jobs: i64,
    stuck_jobs: i64,
    failed_extraction_tasks: i64,
) -> Vec<HealthAction> {
    queue_actions_with_replay(
        failed_pending_observations,
        expired_processing_pending_observations,
        expired_processing_extraction_tasks,
        failed_jobs,
        stuck_jobs,
        failed_extraction_tasks,
        0,
    )
}

pub(crate) fn queue_actions_with_replay(
    failed_pending_observations: i64,
    expired_processing_pending_observations: i64,
    expired_processing_extraction_tasks: i64,
    failed_jobs: i64,
    stuck_jobs: i64,
    failed_extraction_tasks: i64,
    retryable_extraction_replay_ranges: i64,
) -> Vec<HealthAction> {
    let mut actions = Vec::new();

    if failed_pending_observations > 0 {
        actions.push(
            HealthAction::new(count_title(
                failed_pending_observations,
                "failed pending observation",
                "failed pending observations",
            ))
            .command("inspect", "remem pending list-failed --limit 20")
            .command(
                "preview migration prep",
                "remem pending retry-failed --dry-run",
            )
            .command("apply migration prep", "remem pending retry-failed")
            .command("preview replay", "remem pending migrate-legacy --dry-run")
            .command("apply replay", "remem pending migrate-legacy"),
        );
    }

    if expired_processing_pending_observations > 0 {
        actions.push(
            HealthAction::new(count_title(
                expired_processing_pending_observations,
                "expired processing pending observation",
                "expired processing pending observations",
            ))
            .command("inspect counts", "remem status --json")
            .command("preview replay", "remem pending migrate-legacy --dry-run")
            .command("apply replay", "remem pending migrate-legacy"),
        );
    }

    if expired_processing_extraction_tasks > 0 {
        actions.push(
            HealthAction::new(count_title(
                expired_processing_extraction_tasks,
                "expired processing extraction task",
                "expired processing extraction tasks",
            ))
            .command("inspect counts", "remem status --json")
            .command("recover", "remem worker --once"),
        );
    }

    if failed_jobs > 0 {
        actions.push(
            HealthAction::new(count_title(failed_jobs, "failed job", "failed jobs"))
                .command("inspect counts", "remem status --json"),
        );
    }

    if failed_extraction_tasks > 0 {
        actions.push(
            HealthAction::new(count_title(
                failed_extraction_tasks,
                "failed extraction task",
                "failed extraction tasks",
            ))
            .command("inspect counts", "remem status --json"),
        );
    }

    if retryable_extraction_replay_ranges > 0 {
        actions.push(
            HealthAction::new(count_title(
                retryable_extraction_replay_ranges,
                "retryable extraction replay range",
                "retryable extraction replay ranges",
            ))
            .command("inspect", "remem pending list-extraction-ranges --limit 20")
            .command(
                "preview retry",
                "remem pending retry-extraction-ranges --dry-run",
            ),
        );
    }

    if stuck_jobs > 0 {
        actions.push(
            HealthAction::new(count_title(stuck_jobs, "stuck job", "stuck jobs"))
                .command("inspect counts", "remem status --json")
                .command("recover", "remem worker --once"),
        );
    }

    actions
}

pub(crate) fn render_action_block(actions: &[HealthAction]) -> String {
    if actions.is_empty() {
        return String::new();
    }

    let mut output = String::from("Needs attention:\n");
    for action in actions {
        output.push_str(&format!("  - {}\n", action.title));
        for command in &action.commands {
            output.push_str(&format!("    {}: {}\n", command.label, command.command));
        }
    }
    output
}

pub(crate) fn render_inline_hints(actions: &[HealthAction]) -> Option<String> {
    let mut rendered = Vec::new();
    for action in actions {
        for command in &action.commands {
            let hint = format!("{}: `{}`", command.label, command.command);
            if !rendered.contains(&hint) {
                rendered.push(hint);
            }
        }
    }

    (!rendered.is_empty()).then(|| rendered.join("; "))
}

pub(crate) fn worker_once_fallback_human() -> &'static str {
    "when Stop hooks are installed, they run remem worker --once"
}

pub(crate) fn worker_once_fallback_detail() -> &'static str {
    "safe fallback when Stop hooks are installed: `remem worker --once`"
}

fn count_title(count: i64, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}
