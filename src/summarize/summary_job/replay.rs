#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SummaryPayloadOrigin {
    Live,
    Replay,
}

impl SummaryPayloadOrigin {
    pub(super) fn is_replay(self) -> bool {
        matches!(self, Self::Replay)
    }
}

pub(super) fn replay_capture_event_id(
    host: &str,
    project: &str,
    session_id: &str,
    input: &str,
) -> String {
    let seed = format!("{host}\n{project}\n{session_id}\n{input}");
    let digest = crate::db::content_identity_hash(seed.as_bytes());
    let suffix = digest.rsplit(':').next().unwrap_or(digest.as_str());
    format!("session_stop-spill-{suffix}")
}

#[cfg(test)]
mod tests {
    use crate::db::{self, test_support::ScopedTestDataDir};

    use super::SummaryPayloadOrigin;

    #[test]
    fn replay_capture_event_id_is_stable_and_scoped() {
        let first = super::replay_capture_event_id(
            "codex-cli",
            "/tmp/remem",
            "sess-replay",
            r#"{"session_id":"sess-replay","cwd":"/tmp/remem"}"#,
        );
        let again = super::replay_capture_event_id(
            "codex-cli",
            "/tmp/remem",
            "sess-replay",
            r#"{"session_id":"sess-replay","cwd":"/tmp/remem"}"#,
        );
        let other_project = super::replay_capture_event_id(
            "codex-cli",
            "/tmp/other",
            "sess-replay",
            r#"{"session_id":"sess-replay","cwd":"/tmp/remem"}"#,
        );

        assert_eq!(first, again);
        assert_ne!(first, other_project);
        assert!(first.starts_with("session_stop-spill-"));
    }

    #[test]
    fn replay_capture_failure_is_preserved_once_by_replay_layer() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-replay-capture-failure-once");
        let conn = db::open_db()?;
        conn.execute_batch(
            "CREATE TRIGGER fail_session_stop_capture
             BEFORE INSERT ON captured_events
             WHEN NEW.event_type = 'session_stop'
             BEGIN
                 SELECT RAISE(FAIL, 'forced capture failure');
             END;",
        )?;
        let input = serde_json::json!({
            "session_id": "sess-replay-capture-failure",
            "cwd": "/tmp/remem"
        })
        .to_string();
        super::super::spill::spill_summary_hook_payload(
            &input,
            Some("codex-cli"),
            None,
            Some("/tmp/remem"),
            &anyhow::anyhow!("initial spill"),
        )?;

        let replayed =
            super::super::spill::replay_spilled_summary_hook_payloads(&conn, |conn, record| {
                super::super::hook::enqueue_summary_payload(
                    conn,
                    &record.input,
                    record.host.as_deref(),
                    record.profile.as_deref(),
                    SummaryPayloadOrigin::Replay,
                )
            })?;

        assert_eq!(replayed, 0);
        let active = std::fs::read_to_string(super::super::spill::summary_spill_path())?;
        assert_eq!(
            active
                .lines()
                .filter(|line| line.contains("sess-replay-capture-failure"))
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn replay_capture_is_idempotent_when_later_followup_fails() -> anyhow::Result<()> {
        let _test_dir = ScopedTestDataDir::new("summary-replay-capture-idempotent");
        let conn = db::open_db()?;
        conn.execute_batch(
            "CREATE TRIGGER fail_summary_followups
             BEFORE INSERT ON jobs
             BEGIN
                 SELECT RAISE(FAIL, 'forced followup failure');
             END;",
        )?;
        let input = serde_json::json!({
            "session_id": "sess-replay-idempotent",
            "cwd": "/tmp/remem"
        })
        .to_string();
        super::super::spill::spill_summary_hook_payload(
            &input,
            Some("codex-cli"),
            None,
            Some("/tmp/remem"),
            &anyhow::anyhow!("initial spill"),
        )?;

        for _ in 0..2 {
            let replayed = super::super::spill::replay_spilled_summary_hook_payloads(
                &conn,
                |conn, record| {
                    super::super::hook::enqueue_summary_payload(
                        conn,
                        &record.input,
                        record.host.as_deref(),
                        record.profile.as_deref(),
                        SummaryPayloadOrigin::Replay,
                    )
                },
            )?;
            assert_eq!(replayed, 0);
        }

        let event_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM captured_events
             WHERE event_type = 'session_stop'
               AND session_id = 'sess-replay-idempotent'",
            [],
            |row| row.get(0),
        )?;
        let high_watermark_count: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT high_watermark_event_id) FROM extraction_tasks",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(event_count, 1);
        assert_eq!(high_watermark_count, 1);
        Ok(())
    }
}
