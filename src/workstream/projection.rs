use super::WorkStream;

/// Redact sensitive text fields for MCP/CLI output surfaces.
///
/// Keep this out of `map_workstream_row`: matcher identity and context project-scope
/// checks must continue to see canonical stored values.
pub fn redact_workstream_for_output(
    workstream: WorkStream,
    redact: impl Fn(&str) -> String,
) -> WorkStream {
    let redacted_topic = workstream.session_topic.as_deref().map(&redact);
    // Match REST projection: redact topic before label derivation, but keep the raw
    // title as the label fallback input and redact the title field separately.
    let label = crate::memory::session_label::render_from_stored(
        Some(workstream.created_at_epoch),
        workstream.session_intent.as_deref(),
        redacted_topic.as_deref(),
        workstream.session_intent_source.as_deref(),
        Some(&workstream.title),
    );
    WorkStream {
        id: workstream.id,
        project: redact(&workstream.project),
        title: redact(&workstream.title),
        description: workstream.description.as_deref().map(&redact),
        status: workstream.status,
        progress: workstream.progress.as_deref().map(&redact),
        next_action: workstream.next_action.as_deref().map(&redact),
        blockers: workstream.blockers.as_deref().map(&redact),
        created_at_epoch: workstream.created_at_epoch,
        updated_at_epoch: workstream.updated_at_epoch,
        completed_at_epoch: workstream.completed_at_epoch,
        mmdd: label.mmdd,
        session_intent: label.session_intent,
        session_topic: label.session_topic,
        display_label: label.display_label,
        session_intent_source: label.session_intent_source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workstream::WorkStreamStatus;

    fn sample(topic: &str, title: &str, project: &str) -> WorkStream {
        WorkStream {
            id: 1,
            project: project.to_string(),
            title: title.to_string(),
            description: Some("token=desc-secret".to_string()),
            status: WorkStreamStatus::Active,
            progress: Some("token=progress-secret".to_string()),
            next_action: Some("token=next-secret".to_string()),
            blockers: Some("token=blocker-secret".to_string()),
            created_at_epoch: 1735660800,
            updated_at_epoch: 1735660800,
            completed_at_epoch: None,
            mmdd: None,
            session_intent: Some("fix".to_string()),
            session_topic: Some(topic.to_string()),
            display_label: None,
            session_intent_source: Some("summary".to_string()),
        }
    }

    fn redact_token_assignments(text: &str) -> String {
        if text.starts_with("token=") {
            "token=[REDACTED]".to_string()
        } else {
            text.to_string()
        }
    }

    #[test]
    fn output_projection_redacts_topic_before_label_and_text_fields() {
        let projected = redact_workstream_for_output(
            sample(
                "token=mcp-cli-workstream-secret",
                "Safe listing",
                "test/proj",
            ),
            redact_token_assignments,
        );
        assert_eq!(projected.session_topic.as_deref(), Some("token=[REDACTED]"));
        assert_eq!(
            projected.display_label.as_deref(),
            Some("0101｜fix｜token=[REDACTED]")
        );
        assert_eq!(projected.title, "Safe listing");
        assert_eq!(projected.project, "test/proj");
        assert_eq!(projected.description.as_deref(), Some("token=[REDACTED]"));
        assert_eq!(projected.progress.as_deref(), Some("token=[REDACTED]"));
        assert_eq!(projected.next_action.as_deref(), Some("token=[REDACTED]"));
        assert_eq!(projected.blockers.as_deref(), Some("token=[REDACTED]"));
    }

    #[test]
    fn output_projection_preserves_canonical_project_when_redactor_is_identity() {
        let original = sample(
            "Batch text display",
            "Repair listing",
            "/home/u/project2abcd",
        );
        let projected = redact_workstream_for_output(original.clone(), |text| text.to_string());
        assert_eq!(projected.project, original.project);
        assert_eq!(projected.title, original.title);
        assert_eq!(
            projected.session_topic.as_deref(),
            Some("Batch text display")
        );
    }

    #[test]
    fn output_projection_preserves_benign_long_project_paths_with_real_redactor() {
        let project = "/home/u/project2abcd1234567890abcdef12";
        let projected = redact_workstream_for_output(
            sample("Batch text display", "Repair listing", project),
            crate::adapter::common::redact_projected_sensitive_text,
        );
        assert_eq!(projected.project, project);
    }
}
