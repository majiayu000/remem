use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

use crate::memory::raw_archive::{RawIngestReport, TranscriptDrainOptions, SOURCE_ROOT_LOCAL};

#[derive(Debug)]
struct StopTranscriptProbeFailed;

impl std::fmt::Display for StopTranscriptProbeFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Stop transcript identity probe")
    }
}

impl std::error::Error for StopTranscriptProbeFailed {}

pub(super) struct StopTranscript<'a> {
    pub path: &'a str,
    pub byte_limit: Option<u64>,
    pub project: &'a str,
    pub branch: Option<&'a str>,
    pub cwd: &'a str,
}

pub(super) fn drain_with_identity(
    conn: &Connection,
    input: StopTranscript<'_>,
) -> Result<RawIngestReport> {
    let transcript = Path::new(input.path);
    let scan_root = transcript.parent().unwrap_or_else(|| Path::new("."));
    let plan = crate::ingest::session_identity::probe(
        SOURCE_ROOT_LOCAL,
        scan_root,
        transcript,
        Some(input.project),
    )
    .map_err(|error| error.context(StopTranscriptProbeFailed))?;
    let now = chrono::Utc::now().timestamp();
    let identity_id = crate::ingest::session_identity::upsert_claim(conn, &plan, now)
        .context("Stop transcript identity claim")?;
    crate::ingest::session_identity::resolve_fallback_group(
        conn,
        &plan.source_root,
        &plan.fallback_session_id,
    )
    .context("Stop transcript identity resolution")?;
    let identity = crate::ingest::session_identity::load(conn, identity_id)
        .context("load Stop transcript identity")?;
    if identity.status != "active" {
        bail!("Stop transcript identity conflict; raw rows remain unchanged");
    }

    let options = TranscriptDrainOptions {
        transcript_identity_id: Some(identity.id),
        ..TranscriptDrainOptions::default()
    };
    let report = crate::memory::raw_archive::drain_transcript_with_capture_limit(
        conn,
        input.path,
        &identity.canonical_session_id,
        &identity.project,
        plan.branch.as_deref().or(input.branch),
        plan.cwd.as_deref().or(Some(input.cwd)),
        &options,
        input.byte_limit,
    )?;

    let captured_full_file = input
        .byte_limit
        .is_none_or(|limit| i64::try_from(limit).ok() == Some(plan.observed_size_bytes));
    if !report.has_failures() && !report.partial_tail && captured_full_file {
        let index = crate::ingest::session_identity::index_events(
            input.path,
            u64::try_from(plan.observed_size_bytes).unwrap_or(u64::MAX),
        )?;
        crate::ingest::session_identity::record_unfinalized_event_index(
            conn,
            identity.id,
            index,
            now,
        )?;
    }
    Ok(report)
}

pub(super) fn permits_hook_fallback(error: &anyhow::Error) -> bool {
    error.downcast_ref::<StopTranscriptProbeFailed>().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_drain_uses_transcript_metadata_identity() {
        let conn = Connection::open_in_memory().expect("open fixture database");
        crate::migrate::run_migrations(&conn).expect("migrate fixture database");
        let path = std::env::temp_dir().join(format!(
            "remem-gh871-stop-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"sessionId\":\"metadata-session\",",
                "\"cwd\":\"/tmp/project\",\"timestamp\":100,",
                "\"message\":{\"content\":\"hello\"}}\n"
            ),
        )
        .expect("write transcript");
        let path_text = path.to_string_lossy();

        let report = drain_with_identity(
            &conn,
            StopTranscript {
                path: &path_text,
                byte_limit: None,
                project: "hook-project",
                branch: Some("volatile"),
                cwd: "/tmp/project",
            },
        )
        .expect("drain Stop transcript");

        assert_eq!(report.inserted, 1);
        let row: (String, Option<i64>) = conn
            .query_row(
                "SELECT session_id, transcript_identity_id FROM raw_messages",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("load raw occurrence");
        assert_eq!(row.0, "metadata-session");
        assert!(row.1.is_some());
        std::fs::remove_file(path.as_path()).expect("remove transcript");
    }

    #[test]
    fn stop_identity_conflict_does_not_permit_hook_fallback() {
        let conn = Connection::open_in_memory().expect("open fixture database");
        crate::migrate::run_migrations(&conn).expect("migrate fixture database");
        let path = std::env::temp_dir().join(format!(
            "remem-gh871-stop-conflict-{}-{}.jsonl",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let write_claim = |session_id: &str, now: i64| {
            std::fs::write(
                &path,
                serde_json::json!({
                    "type": "user",
                    "sessionId": session_id,
                    "message": {"content": "first"}
                })
                .to_string(),
            )
            .expect("write transcript claim");
            let plan = crate::ingest::session_identity::probe(
                SOURCE_ROOT_LOCAL,
                path.parent().expect("fixture parent"),
                &path,
                Some("/tmp/remem"),
            )
            .expect("probe transcript claim");
            crate::ingest::session_identity::upsert_claim(&conn, &plan, now)
                .expect("persist transcript claim");
            crate::ingest::session_identity::resolve_fallback_group(
                &conn,
                &plan.source_root,
                &plan.fallback_session_id,
            )
            .expect("resolve transcript identity");
        };
        write_claim("canonical-a", 1);
        write_claim("canonical-b", 2);
        let path_text = path.to_string_lossy();

        let error = drain_with_identity(
            &conn,
            StopTranscript {
                path: &path_text,
                byte_limit: None,
                project: "/tmp/remem",
                branch: None,
                cwd: "/tmp/remem",
            },
        )
        .expect_err("identity conflict must remain retryable");

        assert!(format!("{error:#}").contains("identity conflict"));
        assert!(!permits_hook_fallback(&error));
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM raw_messages", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("count raw rows"),
            0
        );
        std::fs::remove_file(path.as_path()).expect("remove transcript");
    }
}
