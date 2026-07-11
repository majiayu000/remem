use std::time::Instant;

use anyhow::{Context, Result};

use crate::db;
use crate::hook_stdin::read_stdin_with_timeout;
use crate::perf::{format_phase_timings, push_elapsed, time_result, PhaseTiming};

use super::super::constants::SUMMARIZE_STDIN_TIMEOUT_MS;
use super::super::input::SummarizeInput;
use super::host::resolve_hook_host;
use super::replay::{replay_capture_event_id, SummaryPayloadOrigin};
use super::spill::{
    replay_spilled_summary_hook_payloads, spill_summary_hook_payload,
    spill_summary_hook_payload_with_git_evidence,
};
use super::worker_launch::{spawn_worker_once_if_idle, WorkerSpawnDecision};

pub async fn summarize(host: Option<&str>, profile: Option<&str>) -> Result<()> {
    let Some(input) = read_stdin_with_timeout(SUMMARIZE_STDIN_TIMEOUT_MS)? else {
        return Ok(());
    };

    summarize_input(&input, host, profile).await
}

pub(super) async fn summarize_input(
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
) -> Result<()> {
    let total_start = Instant::now();
    let mut timings = Vec::new();
    let hook: SummarizeInput = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(err) => {
            crate::log::warn(
                "summarize",
                &format!("invalid hook payload, skipping: {}", err),
            );
            return Ok(());
        }
    };
    if hook.session_id.is_none() {
        return Ok(());
    }
    let host = time_result(&mut timings, "resolve_host", || resolve_hook_host(host))?;
    let cwd = effective_cwd(&hook)?;
    let captured_input = match summary_payload_with_cwd(input, &cwd, profile) {
        Ok(payload) => payload,
        Err(error) => {
            spill_summary_hook_payload(input, Some(&host), profile, Some(&cwd), &error)?;
            return Err(error);
        }
    };
    let prepared_hook: SummarizeInput = serde_json::from_str(&captured_input)?;
    let git_evidence = summary_git_evidence_or_empty(&host, &prepared_hook, &cwd);
    let input = captured_input.as_str();
    let conn = match time_result(&mut timings, "open_db_for_hook", db::open_db_for_hook) {
        Ok(conn) => conn,
        Err(error) => {
            let spill_start = Instant::now();
            let spill_result = spill_summary_hook_payload_with_git_evidence(
                input,
                Some(&host),
                profile,
                Some(&cwd),
                &git_evidence,
                &error,
            );
            push_elapsed(&mut timings, "spill_payload", spill_start);
            let path = spill_result?;
            crate::log::error(
                "summarize",
                &format!(
                    "database open failed; spilled summary hook payload to {}: {}",
                    path.display(),
                    error
                ),
            );
            push_elapsed(&mut timings, "hook_total", total_start);
            log_summary_hook_timing("db_open_failed", &host, &timings);
            return Err(error);
        }
    };
    time_result(&mut timings, "enqueue_summary_payload", || {
        enqueue_summary_payload_with_git_evidence(
            &conn,
            input,
            Some(&host),
            profile,
            SummaryPayloadOrigin::Live,
            Some(&git_evidence),
        )
    })?;
    let current_identity =
        SummaryPayloadIdentity::from_hook(&host, &hook, &cwd, &db::project_from_cwd(&cwd));
    if let Err(error) = time_result(&mut timings, "spill_replay", || {
        replay_spilled_summary_hook_payloads(&conn, |conn, record| {
            if summary_payload_identity(&record.input, record.host.as_deref())?.as_ref()
                == Some(&current_identity)
            {
                crate::log::info(
                    "summarize",
                    &format!(
                        "skipped spilled summary hook payload for current identity host={} project={} session={}",
                        current_identity.host, current_identity.project, current_identity.session_id
                    ),
                );
                return record_replayed_git_evidence_only(conn, record);
            }
            enqueue_summary_payload_with_git_evidence(
                conn,
                &record.input,
                record.host.as_deref(),
                record.profile.as_deref(),
                SummaryPayloadOrigin::Replay,
                Some(&record.git_evidence),
            )
        })
    }) {
        crate::log::error(
            "summarize",
            &format!("summary hook spill replay failed; continuing with current payload: {error}"),
        );
    }
    match time_result(&mut timings, "worker_once_spawn", || {
        spawn_worker_once_if_idle(&conn)
    }) {
        Ok(WorkerSpawnDecision::Spawned) => {
            crate::log::info("summarize", "worker --once spawned");
        }
        Ok(WorkerSpawnDecision::SkippedHealthyWorker) => {
            crate::log::info("summarize", "worker heartbeat healthy; skip worker --once");
        }
        Ok(WorkerSpawnDecision::SkippedLaunchInProgress) => {
            crate::log::info(
                "summarize",
                "worker --once launch already in progress; skip spawn",
            );
        }
        Err(error) => {
            crate::log::error(
                "summarize",
                &format!("summary jobs queued but worker --once spawn failed: {error}"),
            );
        }
    }
    push_elapsed(&mut timings, "hook_total", total_start);
    log_summary_hook_timing("queued", &host, &timings);
    Ok(())
}

#[cfg(test)]
pub(super) fn enqueue_summary_payload(
    conn: &rusqlite::Connection,
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
    origin: SummaryPayloadOrigin,
) -> Result<()> {
    enqueue_summary_payload_with_git_evidence(conn, input, host, profile, origin, None)
}

pub(super) fn enqueue_summary_payload_with_git_evidence(
    conn: &rusqlite::Connection,
    input: &str,
    host: Option<&str>,
    profile: Option<&str>,
    origin: SummaryPayloadOrigin,
    provided_git_evidence: Option<&[crate::git_util::GitCommitEvidence]>,
) -> Result<()> {
    let hook: SummarizeInput = serde_json::from_str(input)?;
    let Some(session_id) = &hook.session_id else {
        return Ok(());
    };
    let cwd = effective_cwd(&hook)?;
    let project = db::project_from_cwd(&cwd);
    let host = resolve_hook_host(host)?;
    let summary_payload = match summary_payload_with_cwd(input, &cwd, profile) {
        Ok(payload) => payload,
        Err(error) => {
            if origin.is_replay() {
                crate::log::error(
                    "summarize",
                    &format!(
                        "replayed Stop payload preparation failed; replay layer will preserve it: {error}"
                    ),
                );
            } else {
                let path =
                    spill_summary_hook_payload(input, Some(&host), profile, Some(&cwd), &error)?;
                crate::log::error(
                    "summarize",
                    &format!(
                        "Stop payload preparation failed; spilled summary hook payload to {}: {error}",
                        path.display()
                    ),
                );
            }
            return Err(error);
        }
    };
    let prepared_hook: SummarizeInput = serde_json::from_str(&summary_payload)?;
    let discovered_git_evidence;
    let git_evidence = if let Some(provided) = provided_git_evidence {
        provided
    } else {
        discovered_git_evidence = summary_git_evidence_or_empty(&host, &prepared_hook, &cwd);
        discovered_git_evidence.as_slice()
    };
    let replay_event_id = origin
        .is_replay()
        .then(|| replay_capture_event_id(&host, &project, session_id, &summary_payload));
    if let Err(error) = record_summary_capture_event(
        conn,
        &host,
        session_id,
        &project,
        &cwd,
        &summary_payload,
        replay_event_id.as_deref(),
        git_evidence,
    ) {
        let error_text = error.to_string();
        if origin.is_replay() {
            crate::log::error(
                "summarize",
                &format!(
                    "replayed capture ledger record failed; replay layer will preserve summary hook payload and skip follow-up jobs: {error_text}"
                ),
            );
        } else {
            let path = spill_summary_hook_payload_with_git_evidence(
                input,
                Some(&host),
                profile,
                Some(&cwd),
                git_evidence,
                &error,
            )?;
            crate::log::error(
                "summarize",
                &format!(
                    "capture ledger record failed; spilled summary hook payload to {} and skipped follow-up jobs: {}",
                    path.display(),
                    error_text
                ),
            );
        }
        anyhow::bail!(error_text);
    }
    let current_branch = db::detect_git_branch(&cwd);
    super::side_effects::run_stop_hook_side_effects(
        conn,
        &host,
        &prepared_hook,
        session_id,
        &project,
        &cwd,
        current_branch.as_deref(),
        false,
    )?;
    Ok(())
}

fn summary_git_evidence(
    host: &str,
    hook: &SummarizeInput,
    cwd: &str,
) -> Result<Vec<crate::git_util::GitCommitEvidence>> {
    if host != "codex-cli" {
        return Ok(Vec::new());
    }
    let (Some(transcript_path), Some(byte_limit)) =
        (hook.transcript_path.as_deref(), hook.transcript_byte_len)
    else {
        return Ok(Vec::new());
    };
    crate::git_evidence::from_codex_transcript(transcript_path, byte_limit, cwd)
}

fn summary_git_evidence_or_empty(
    host: &str,
    hook: &SummarizeInput,
    cwd: &str,
) -> Vec<crate::git_util::GitCommitEvidence> {
    match summary_git_evidence(host, hook, cwd) {
        Ok(evidence) => evidence,
        Err(error) => {
            crate::log::error(
                "summarize",
                &format!(
                    "commit evidence extraction failed; preserving Stop capture without commit evidence host={host} session={}: {error:#}",
                    hook.session_id.as_deref().unwrap_or("unknown")
                ),
            );
            Vec::new()
        }
    }
}

fn effective_cwd(hook: &SummarizeInput) -> Result<String> {
    if let Some(cwd) = hook.cwd.as_deref().filter(|cwd| !cwd.trim().is_empty()) {
        return Ok(cwd.to_string());
    }
    Ok(std::env::current_dir()?.display().to_string())
}

fn summary_payload_with_cwd(input: &str, cwd: &str, profile: Option<&str>) -> Result<String> {
    let mut payload: serde_json::Value = serde_json::from_str(input)?;
    let Some(obj) = payload.as_object_mut() else {
        return Ok(input.to_string());
    };
    let needs_cwd = obj
        .get("cwd")
        .and_then(|value| value.as_str())
        .is_none_or(|value| value.trim().is_empty());
    if needs_cwd {
        obj.insert(
            "cwd".to_string(),
            serde_json::Value::String(cwd.to_string()),
        );
    }
    let transcript_path = obj
        .get("transcript_path")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    if obj
        .get("transcript_byte_len")
        .and_then(serde_json::Value::as_u64)
        .is_none()
    {
        if let Some(transcript_path) = transcript_path {
            let metadata = std::fs::metadata(&transcript_path)
                .with_context(|| format!("snapshot transcript length path={transcript_path}"))?;
            obj.insert(
                "transcript_byte_len".to_string(),
                serde_json::Value::Number(metadata.len().into()),
            );
        }
    }
    if let Some(profile) = clean_optional(profile) {
        obj.insert(
            crate::runtime_config::MEMORY_AI_PROFILE_FIELD.to_string(),
            serde_json::Value::String(profile),
        );
    }
    Ok(serde_json::to_string(&payload)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SummaryPayloadIdentity {
    host: String,
    session_id: String,
    project: String,
}

impl SummaryPayloadIdentity {
    fn from_hook(host: &str, hook: &SummarizeInput, cwd: &str, project: &str) -> Self {
        Self {
            host: host.to_string(),
            session_id: hook.session_id.clone().unwrap_or_default(),
            project: if project.trim().is_empty() {
                db::project_from_cwd(cwd)
            } else {
                project.to_string()
            },
        }
    }
}

fn summary_payload_identity(
    input: &str,
    host: Option<&str>,
) -> Result<Option<SummaryPayloadIdentity>> {
    let hook: SummarizeInput = serde_json::from_str(input)?;
    let Some(session_id) = hook.session_id.clone() else {
        return Ok(None);
    };
    let host = resolve_hook_host(host)?;
    let cwd = effective_cwd(&hook)?;
    let project = db::project_from_cwd(&cwd);
    Ok(Some(SummaryPayloadIdentity {
        host,
        session_id,
        project,
    }))
}

fn record_summary_capture_event(
    conn: &rusqlite::Connection,
    host: &str,
    session_id: &str,
    project: &str,
    cwd: &str,
    content: &str,
    event_id: Option<&str>,
    git_evidence: &[crate::git_util::GitCommitEvidence],
) -> Result<()> {
    db::record_captured_event_with_id_and_reference_time_and_git_evidence(
        conn,
        &db::CaptureEventInput {
            host,
            session_id,
            project,
            cwd: Some(cwd),
            event_type: "session_stop",
            role: None,
            tool_name: None,
            content,
            task_kind: Some(db::ExtractionTaskKind::SessionRollup),
        },
        event_id,
        None,
        git_evidence,
    )?;
    Ok(())
}

fn record_replayed_git_evidence_only(
    conn: &rusqlite::Connection,
    record: &super::spill::SummaryHookSpillRecord,
) -> Result<()> {
    if record.git_evidence.is_empty() {
        return Ok(());
    }
    let hook: SummarizeInput = serde_json::from_str(&record.input)?;
    let Some(session_id) = hook.session_id.as_deref() else {
        return Ok(());
    };
    let cwd = effective_cwd(&hook)?;
    let project = db::project_from_cwd(&cwd);
    let host = resolve_hook_host(record.host.as_deref())?;
    let shas = record
        .git_evidence
        .iter()
        .map(|evidence| evidence.metadata.sha.as_str())
        .collect::<Vec<_>>();
    let content = serde_json::json!({
        "source": "replayed_stop_commit_evidence",
        "commit_shas": shas,
    })
    .to_string();
    let event_id = db::unique_capture_event_id("commit_evidence", &content);
    db::record_captured_event_with_id_and_reference_time_and_git_evidence(
        conn,
        &db::CaptureEventInput {
            host: &host,
            session_id,
            project: &project,
            cwd: Some(&cwd),
            event_type: "commit_evidence",
            role: None,
            tool_name: None,
            content: &content,
            task_kind: Some(db::ExtractionTaskKind::ObservationExtract),
        },
        Some(&event_id),
        None,
        &record.git_evidence,
    )?;
    Ok(())
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn log_summary_hook_timing(status: &str, host: &str, timings: &[PhaseTiming]) {
    crate::log::info(
        "summarize-perf",
        &format!(
            "status={} host={} timings=[{}]",
            status,
            host,
            format_phase_timings(timings)
        ),
    );
}

#[cfg(test)]
#[path = "hook/tests.rs"]
mod tests;
