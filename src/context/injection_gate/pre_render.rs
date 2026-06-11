use super::*;
use rusqlite::OptionalExtension;

pub(in crate::context) fn pre_render_context_gate(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
) -> Option<ContextGateDecision> {
    if !host_is_gated(invocation.host) {
        return None;
    }
    if resolve_gate_mode(invocation.gate_mode.as_deref()) != ContextGateMode::Strict {
        return None;
    }
    if invocation.force || source_requires_fresh_emission(invocation.source.as_deref()) {
        return None;
    }
    if !has_trusted_gate_identity(invocation) {
        return None;
    }

    let key = injection_key(invocation);
    let now = chrono::Utc::now().timestamp();
    let row = match load_retained_gate_row(
        conn,
        invocation.host.as_env_value(),
        &key,
        retention_cutoff_epoch(now),
    ) {
        Ok(row) => row,
        Err(error) => {
            crate::log::warn(
                "context-gate",
                &format!("pre_render_skip reason=gate_read error={}", error),
            );
            return None;
        }
    };

    let row = row?;
    crate::log::info(
        "context-gate",
        &format!(
            "pre_render_suppress host={} key={} reason=strict hash={} project={}",
            invocation.host.as_env_value(),
            key,
            row.context_hash,
            invocation.project
        ),
    );
    Some(gate_decision(
        String::new(),
        ContextGateAction::Suppressed,
        "strict_pre_render",
        &key,
        &row.context_hash,
        "suppressed",
    ))
}

fn load_retained_gate_row(
    conn: &rusqlite::Connection,
    host: &str,
    key: &str,
    cutoff_epoch: i64,
) -> Result<Option<GateRow>> {
    conn.query_row(
        "SELECT context_hash, last_emitted_epoch
         FROM context_injections
         WHERE host = ?1
           AND injection_key = ?2
           AND updated_at_epoch >= ?3",
        rusqlite::params![host, key, cutoff_epoch],
        |row| {
            Ok(GateRow {
                context_hash: row.get(0)?,
                last_emitted_epoch: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::ffi::OsString;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn gate_invocation(session_id: Option<&str>) -> ContextInvocation {
        ContextInvocation {
            cwd: "/tmp/remem".to_string(),
            project: "/tmp/remem".to_string(),
            session_id: session_id.map(str::to_string),
            transcript_path: Some("/tmp/remem.jsonl".to_string()),
            source: None,
            host: HostKind::CodexCli,
            use_colors: false,
            debug: false,
            force: false,
            gate_mode: None,
        }
    }

    #[test]
    fn strict_pre_render_suppresses_existing_session_without_render_output() -> anyhow::Result<()> {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render");
        let mut invocation = gate_invocation(Some("sess-pre-render"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_context_gate(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let Some(decision) = pre_render_context_gate(&conn, &invocation) else {
            anyhow::bail!("existing strict gate row should suppress before render");
        };
        assert_eq!(decision.action, ContextGateAction::Suppressed);
        assert_eq!(decision.reason, "strict_pre_render");
        assert!(decision.output.is_empty());
        assert_eq!(decision.output_mode, Some("suppressed"));
        assert_eq!(decision.context_hash, first.context_hash);
        Ok(())
    }

    #[test]
    fn strict_pre_render_does_not_suppress_first_session_context() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-first");
        let mut invocation = gate_invocation(Some("sess-pre-render-first"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        assert!(pre_render_context_gate(&conn, &invocation).is_none());
        Ok(())
    }

    #[test]
    fn strict_pre_render_does_not_suppress_force_or_clear() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-clear");
        let mut invocation = gate_invocation(Some("sess-pre-render-clear"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_context_gate(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        invocation.force = true;
        assert!(pre_render_context_gate(&conn, &invocation).is_none());

        invocation.force = false;
        invocation.source = Some("clear".to_string());
        assert!(pre_render_context_gate(&conn, &invocation).is_none());
        Ok(())
    }

    #[test]
    fn strict_pre_render_respects_retention_cutoff() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-retention");
        let _retention = ScopedEnvVar::set("REMEM_CONTEXT_GATE_RETENTION_DAYS", "30");
        let mut invocation = gate_invocation(Some("sess-pre-render-retention"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_context_gate(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let old_epoch = chrono::Utc::now().timestamp().saturating_sub(31 * 86_400);
        conn.execute(
            "UPDATE context_injections
             SET updated_at_epoch = ?1, last_emitted_epoch = ?1
             WHERE host = ?2 AND injection_key = ?3",
            params![
                old_epoch,
                invocation.host.as_env_value(),
                injection_key(&invocation)
            ],
        )?;

        assert!(pre_render_context_gate(&conn, &invocation).is_none());
        Ok(())
    }
}
