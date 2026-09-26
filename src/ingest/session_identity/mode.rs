//! Versioned provenance transition; legacy evidence is used only for an existing v0 claim.
use anyhow::{bail, Result};
use serde_json::Value;

use super::{CodexSessionMode, TranscriptPlan};

pub(super) fn legacy(payload: Option<&Value>) -> CodexSessionMode {
    let field = |key| {
        payload
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
    };
    match field("thread_source") {
        Some("subagent") => CodexSessionMode::Subagent,
        Some("automation") => CodexSessionMode::Unattended,
        _ => match field("originator") {
            Some("codex-tui" | "Codex Desktop" | "codex_cli_rs" | "codex_work_desktop") => {
                CodexSessionMode::Interactive
            }
            Some("codex_exec" | "symphony-orchestrator") => CodexSessionMode::Unattended,
            _ => CodexSessionMode::Unknown,
        },
    }
}

pub(super) fn resolve(stored: &str, version: i64, plan: &TranscriptPlan) -> Result<(String, i64)> {
    let proposed = plan.session_mode.as_str();
    let legacy = plan.legacy_session_mode.as_str();
    let conflict =
        |observed: &str| stored != "unknown" && observed != "unknown" && stored != observed;
    // A legacy row must still agree with the legacy evidence. Policy upgrades
    // cannot excuse changed native provenance, even if the new policy agrees.
    if (version == 0 && conflict(legacy))
        || (conflict(proposed) && !(version == 0 && legacy == stored))
    {
        bail!(
            "transcript session-mode provenance conflict for {:?}: stored mode is {:?}, proposed mode is {:?}, legacy evidence is {:?}, classifier version is {}",
            plan.transcript_path, stored, proposed, legacy, version
        );
    }
    if proposed == "unknown" && stored != "unknown" {
        // A short prefix or missing metadata must not consume the upgrade.
        return Ok((stored.to_string(), version));
    }
    Ok((proposed.to_string(), 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::InstallHost;
    use crate::ingest::session_identity::{probe_with_host, upsert_claim};

    // Catches alternate policy transitions, especially legacy-unknown adoption and v1 conflicts.
    #[test]
    fn classification_policy_transition_matrix() {
        let path = super::super::tests::temp_transcript("mode-policy", "{}\n");
        let mut plan = probe_with_host(
            InstallHost::CodexCli,
            "local",
            path.parent().unwrap(),
            &path,
            None,
            None,
        )
        .unwrap();
        for (stored, version, legacy, shared, expected) in [
            (
                "interactive",
                0,
                "interactive",
                "unattended",
                Some(("unattended", 1)),
            ),
            (
                "interactive",
                0,
                "interactive",
                "subagent",
                Some(("subagent", 1)),
            ),
            (
                "interactive",
                0,
                "unknown",
                "interactive",
                Some(("interactive", 1)),
            ),
            (
                "interactive",
                0,
                "unknown",
                "unknown",
                Some(("interactive", 0)),
            ),
            (
                "interactive",
                0,
                "interactive",
                "unknown",
                Some(("interactive", 0)),
            ),
            ("interactive", 0, "unknown", "unattended", None),
            ("interactive", 0, "unattended", "interactive", None),
            ("interactive", 1, "interactive", "unattended", None),
            (
                "interactive",
                1,
                "unattended",
                "interactive",
                Some(("interactive", 1)),
            ),
            (
                "unknown",
                0,
                "interactive",
                "unattended",
                Some(("unattended", 1)),
            ),
            (
                "unknown",
                1,
                "interactive",
                "unattended",
                Some(("unattended", 1)),
            ),
        ] {
            plan.legacy_session_mode = legacy.into();
            plan.session_mode = shared.into();
            let actual = resolve(stored, version, &plan);
            match expected {
                Some((mode, version)) => assert_eq!(actual.unwrap(), (mode.into(), version)),
                None => assert!(actual
                    .unwrap_err()
                    .to_string()
                    .contains("session-mode provenance conflict")),
            }
        }
        std::fs::remove_file(path).unwrap();
    }

    // Catches consuming a deferred upgrade on an unknown prefix and bypassing host guards.
    #[test]
    fn incomplete_legacy_evidence_defers_upgrade_and_host_conflicts_still_fail() {
        let conn = super::super::tests::setup_identity_db();
        let path = super::super::tests::temp_transcript("deferred-mode", "{}\n");
        let mut plan = probe_with_host(
            InstallHost::CodexCli,
            "local",
            path.parent().unwrap(),
            &path,
            None,
            None,
        )
        .unwrap();
        let id = upsert_claim(&conn, &plan, 1).unwrap();
        conn.execute(
            "UPDATE raw_session_identities SET session_mode='interactive', session_mode_version=0",
            [],
        )
        .unwrap();
        assert_eq!(
            resolve("interactive", 0, &plan).unwrap(),
            ("interactive".into(), 0)
        );
        plan.session_mode = "unattended".into();
        assert!(resolve("interactive", 0, &plan)
            .unwrap_err()
            .to_string()
            .contains("session-mode provenance conflict"));
        plan.legacy_session_mode = "interactive".into();
        plan.host = Some(InstallHost::ClaudeCode);
        assert!(upsert_claim(&conn, &plan, 2)
            .unwrap_err()
            .to_string()
            .contains("host provenance conflict"));
        assert_eq!(conn.query_row("SELECT session_mode,session_mode_version,last_seen_at_epoch FROM raw_session_identities WHERE id=?1", [id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap(), ("interactive".into(),0,1));
        std::fs::remove_file(path).unwrap();
    }
}
