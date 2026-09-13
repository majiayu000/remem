use super::WorkStream;

/// Redact sensitive text fields for MCP/CLI output surfaces.
///
/// Keep this out of `map_workstream_row`: matcher identity and context project-scope
/// checks must continue to see canonical stored values.
///
/// Call sites (MCP/CLI) pass both redactors so this module never depends on
/// `adapter`. Use a path-preserving project redactor for `project` and a strict
/// projection redactor for every other text field.
pub(crate) fn redact_workstream_for_output(
    workstream: WorkStream,
    redact: impl Fn(&str) -> String,
    redact_project: impl Fn(&str) -> String,
) -> WorkStream {
    // Render from the already-validated stored topic first. Redacting before
    // `render_from_stored` can expand short credentials past TOPIC_MAX_CHARS and
    // then null out session_topic/display_label via normalize_topic.
    let label = crate::memory::session_label::render_from_stored(
        Some(workstream.created_at_epoch),
        workstream.session_intent.as_deref(),
        workstream.session_topic.as_deref(),
        workstream.session_intent_source.as_deref(),
        Some(&workstream.title),
    );
    WorkStream {
        id: workstream.id,
        project: redact_project(&workstream.project),
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
        session_topic: label.session_topic.as_deref().map(&redact),
        display_label: label.display_label.as_deref().map(&redact),
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
            session_intent: Some("FIX".to_string()),
            session_topic: Some(topic.to_string()),
            display_label: None,
            session_intent_source: Some("summary".to_string()),
        }
    }

    fn redact_token_assignments(text: &str) -> String {
        text.replace("token=mcp-cli-workstream-secret", "token=[REDACTED]")
            .replace("token=desc-secret", "token=[REDACTED]")
            .replace("token=progress-secret", "token=[REDACTED]")
            .replace("token=next-secret", "token=[REDACTED]")
            .replace("token=blocker-secret", "token=[REDACTED]")
    }

    #[test]
    fn output_projection_redacts_rendered_topic_and_label_fields() {
        let projected = redact_workstream_for_output(
            sample(
                "token=mcp-cli-workstream-secret",
                "Safe listing",
                "test/proj",
            ),
            redact_token_assignments,
            |text| text.to_string(),
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
    fn output_projection_keeps_near_limit_topic_after_redaction_expands() {
        // 72 filler chars + " token=x" (8) = 80 chars — valid at storage time.
        // Keep a whitespace boundary so inline redaction still sees key `token`.
        let topic = format!("{} token=x", "n".repeat(72));
        assert_eq!(
            topic.chars().count(),
            crate::memory::session_label::TOPIC_MAX_CHARS
        );
        let projected = redact_workstream_for_output(
            sample(&topic, "Safe listing", "test/proj"),
            crate::adapter::common::redact_projected_sensitive_text,
            crate::adapter::common::redact_projected_project_text,
        );
        let expected_topic = format!("{} token=[REDACTED]", "n".repeat(72));
        assert_eq!(
            projected.session_topic.as_deref(),
            Some(expected_topic.as_str())
        );
        assert!(
            projected.display_label.is_some(),
            "display_label must stay present after placeholder expansion"
        );
        assert!(
            !projected
                .display_label
                .as_deref()
                .unwrap_or_default()
                .contains("token=x"),
            "{:?}",
            projected.display_label
        );
    }

    #[test]
    fn output_projection_preserves_canonical_project_when_redactor_is_identity() {
        let original = sample(
            "Batch text display",
            "Repair listing",
            "/home/u/project2abcd",
        );
        let projected = redact_workstream_for_output(
            original.clone(),
            |text| text.to_string(),
            |text| text.to_string(),
        );
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
            crate::adapter::common::redact_projected_project_text,
        );
        assert_eq!(projected.project, project);
    }

    #[test]
    fn output_projection_redacts_filesystem_token_outside_project_field() {
        let path = "/tmp/Abcdef0123456789Abcdef0123456789";
        let projected = redact_workstream_for_output(
            sample("Batch text display", path, "test/proj"),
            crate::adapter::common::redact_projected_sensitive_text,
            crate::adapter::common::redact_projected_project_text,
        );
        assert_eq!(projected.title, "[REDACTED]");
        assert_eq!(projected.project, "test/proj");
    }

    #[test]
    fn output_projection_applies_caller_project_redactor() {
        let projected = redact_workstream_for_output(
            sample("Batch text display", "Repair listing", "token=proj-secret"),
            |text| text.to_string(),
            |text| text.replace("token=proj-secret", "token=[REDACTED]"),
        );
        assert_eq!(projected.project, "token=[REDACTED]");
        assert_eq!(projected.title, "Repair listing");
    }
}
