use rusqlite::Connection;

use super::CommitSessionLink;

/// Re-scan a linked summary before exposing it through git/MCP trace output.
/// A poisoned, unacknowledged summary is quarantined and withheld (GH-855).
pub(super) fn gate_link_summary(
    conn: &Connection,
    link: &mut CommitSessionLink,
    summary_id: Option<i64>,
) {
    let (Some(summary), Some(summary_id)) = (link.summary.as_ref(), summary_id) else {
        return;
    };
    let injectable = crate::db::summary_poisoning::summary_injectable(
        conn,
        summary_id,
        &[
            ("request", summary.request.as_deref()),
            ("completed", summary.completed.as_deref()),
            ("decisions", summary.decisions.as_deref()),
            ("learned", summary.learned.as_deref()),
            ("next_steps", summary.next_steps.as_deref()),
            ("preferences", summary.preferences.as_deref()),
        ],
        "git_trace",
    );
    if !injectable {
        link.summary = None;
    }
}
