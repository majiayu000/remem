use super::*;

pub(in crate::context) fn pre_render_context_gate(
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
    let conn = match crate::db::open_db_read_only() {
        Ok(conn) => conn,
        Err(error) => {
            crate::log::warn(
                "context-gate",
                &format!("pre_render_skip reason=open_db_read_only error={}", error),
            );
            return None;
        }
    };
    let row = match load_gate_row(&conn, invocation.host.as_env_value(), &key) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn strict_pre_render_suppresses_existing_session_without_render_output() {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render");
        let mut invocation = gate_invocation(Some("sess-pre-render"));
        invocation.gate_mode = Some("strict".to_string());

        let first = apply_context_gate(
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let decision = pre_render_context_gate(&invocation)
            .expect("existing strict gate row should suppress before render");
        assert_eq!(decision.action, ContextGateAction::Suppressed);
        assert_eq!(decision.reason, "strict_pre_render");
        assert!(decision.output.is_empty());
        assert_eq!(decision.output_mode, Some("suppressed"));
        assert_eq!(decision.context_hash, first.context_hash);
    }

    #[test]
    fn strict_pre_render_does_not_suppress_first_session_context() {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-first");
        let mut invocation = gate_invocation(Some("sess-pre-render-first"));
        invocation.gate_mode = Some("strict".to_string());

        assert!(pre_render_context_gate(&invocation).is_none());
    }

    #[test]
    fn strict_pre_render_does_not_suppress_force_or_clear() {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-clear");
        let mut invocation = gate_invocation(Some("sess-pre-render-clear"));
        invocation.gate_mode = Some("strict".to_string());

        let first = apply_context_gate(
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        );
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        invocation.force = true;
        assert!(pre_render_context_gate(&invocation).is_none());

        invocation.force = false;
        invocation.source = Some("clear".to_string());
        assert!(pre_render_context_gate(&invocation).is_none());
    }
}
