use std::collections::HashSet;

use crate::workstream::WorkStream;

use super::types::SessionSummaryBrief;

const MAX_QUERY_CHARS: usize = 512;
const MAX_BRANCH_SEGMENTS: usize = 4;
const MAX_COMMIT_SIGNALS: usize = 3;
const MAX_WORKSTREAM_SIGNALS: usize = 3;
const MAX_SUMMARY_SIGNALS: usize = 2;

pub(super) fn build_implicit_context_query(
    project: &str,
    current_branch: Option<&str>,
    commit_messages: &[String],
    summaries: &[SessionSummaryBrief],
    workstreams: &[WorkStream],
) -> Option<String> {
    let mut builder = QueryBuilder::default();
    builder.push_signal(project.rsplit('/').next().unwrap_or(project));
    if let Some(branch) = current_branch {
        builder.push_signal(branch);
        for segment in branch
            .split(|ch: char| !ch.is_ascii_alphanumeric())
            .filter(|segment| segment.len() >= 3)
            .take(MAX_BRANCH_SEGMENTS)
        {
            builder.push_signal(segment);
        }
    }
    for message in commit_messages.iter().take(MAX_COMMIT_SIGNALS) {
        builder.push_signal(message);
    }
    for workstream in workstreams.iter().take(MAX_WORKSTREAM_SIGNALS) {
        builder.push_signal(&workstream.title);
        builder.push_signal(workstream.next_action.as_deref().unwrap_or_default());
        builder.push_signal(workstream.blockers.as_deref().unwrap_or_default());
    }
    for summary in summaries.iter().take(MAX_SUMMARY_SIGNALS) {
        builder.push_signal(&summary.request);
        builder.push_signal(summary.completed.as_deref().unwrap_or_default());
    }
    builder.build()
}

#[derive(Default)]
struct QueryBuilder {
    parts: Vec<String>,
    seen: HashSet<String>,
    char_count: usize,
}

impl QueryBuilder {
    fn push_signal(&mut self, value: &str) {
        let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
        let trimmed = normalized.trim();
        if trimmed.len() < 2 {
            return;
        }
        let key = trimmed.to_ascii_lowercase();
        if !self.seen.insert(key) {
            return;
        }
        let remaining = MAX_QUERY_CHARS.saturating_sub(self.char_count);
        if remaining == 0 {
            return;
        }
        let part = truncate_chars(trimmed, remaining);
        if part.is_empty() {
            return;
        }
        self.char_count += part.chars().count() + 1;
        self.parts.push(part);
    }

    fn build(self) -> Option<String> {
        (!self.parts.is_empty()).then(|| self.parts.join(" "))
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use crate::workstream::{WorkStream, WorkStreamStatus};

    use super::*;

    #[test]
    fn implicit_query_uses_project_branch_workstream_and_summary_signals() {
        let summaries = vec![SessionSummaryBrief {
            request: "Investigate SQLCipher context recall".to_string(),
            completed: Some("Verified retrieval injection path".to_string()),
            created_at_epoch: 1,
        }];
        let workstreams = vec![WorkStream {
            id: 1,
            project: "/tmp/remem".to_string(),
            title: "Context retrieval".to_string(),
            description: None,
            status: WorkStreamStatus::Active,
            progress: None,
            next_action: Some("Wire hybrid search into SessionStart".to_string()),
            blockers: None,
            created_at_epoch: 1,
            updated_at_epoch: 1,
            completed_at_epoch: None,
        }];

        let query = build_implicit_context_query(
            "/tmp/remem",
            Some("fix/context-retrieval"),
            &["Preserve SQLCipher retrieval evidence".to_string()],
            &summaries,
            &workstreams,
        )
        .expect("query");

        assert!(query.contains("remem"));
        assert!(query.contains("context"));
        assert!(query.contains("retrieval"));
        assert!(query.contains("SessionStart"));
        assert!(query.contains("SQLCipher"));
        assert!(query.contains("Preserve"));
    }
}
