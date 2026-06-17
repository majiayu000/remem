use super::*;
use rusqlite::OptionalExtension;

pub(in crate::context) fn pre_render_context_gate(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    request: &ContextRequest,
    policy: &ContextPolicy,
    debug_enabled: bool,
) -> ContextGatePrecheckResult {
    if !host_is_gated(invocation.host) {
        return ContextGatePrecheckResult::off();
    }
    if resolve_gate_mode(invocation.gate_mode.as_deref()) != ContextGateMode::Strict {
        return ContextGatePrecheckResult::off();
    }
    if invocation.force || source_requires_fresh_emission(invocation.source.as_deref()) {
        return ContextGatePrecheckResult::off();
    }
    if !has_trusted_gate_identity(invocation) {
        return ContextGatePrecheckResult::off();
    }
    if debug_enabled {
        return ContextGatePrecheckResult::off();
    }
    let has_data_version = match data_version::context_injections_has_data_version(conn) {
        Ok(value) => value,
        Err(error) => {
            crate::log::warn(
                "context-gate",
                &format!("pre_render_skip reason=schema_check error={}", error),
            );
            return ContextGatePrecheckResult::miss(None);
        }
    };
    if !has_data_version {
        return ContextGatePrecheckResult::miss(None);
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
            return ContextGatePrecheckResult::miss(None);
        }
    };

    let Some(row) = row else {
        return ContextGatePrecheckResult::miss(None);
    };
    if row.data_version.is_none() {
        return ContextGatePrecheckResult::miss(None);
    }
    let current_data_version =
        match data_version_hint::compute_data_version_hint(conn, request, invocation, policy) {
            Ok(value) => value,
            Err(error) => {
                crate::log::warn(
                    "context-gate",
                    &format!("pre_render_skip reason=data_version error={}", error),
                );
                return ContextGatePrecheckResult::miss(None);
            }
        };

    if row.data_version.as_deref() != Some(current_data_version.as_str()) {
        return ContextGatePrecheckResult::miss(Some(current_data_version));
    }
    suppress_matching_row(conn, invocation, row, Some(current_data_version))
}

pub(in crate::context) struct ContextGatePrecheckResult {
    pub(in crate::context) decision: Option<ContextGateDecision>,
    pub(in crate::context) precheck: ContextGatePrecheck,
    pub(in crate::context) data_version: Option<String>,
}

impl ContextGatePrecheckResult {
    fn off() -> Self {
        Self {
            decision: None,
            precheck: ContextGatePrecheck::Off,
            data_version: None,
        }
    }

    fn miss(data_version: Option<String>) -> Self {
        Self {
            decision: None,
            precheck: ContextGatePrecheck::Miss,
            data_version,
        }
    }

    fn hit(decision: ContextGateDecision, data_version: String) -> Self {
        Self {
            decision: Some(decision),
            precheck: ContextGatePrecheck::Hit,
            data_version: Some(data_version),
        }
    }
}

fn suppress_matching_row(
    conn: &rusqlite::Connection,
    invocation: &ContextInvocation,
    row: GateRow,
    data_version: Option<String>,
) -> ContextGatePrecheckResult {
    let Some(data_version) = data_version else {
        return ContextGatePrecheckResult::miss(None);
    };
    let key = injection_key(invocation);
    let now = chrono::Utc::now().timestamp();
    if !fallback_cooldown_allows_suppression(invocation, &row, now) {
        return ContextGatePrecheckResult::miss(Some(data_version));
    }
    if let Err(error) = record_suppression(conn, invocation, &key, Some(&data_version), now) {
        crate::log::warn(
            "context-gate",
            &format!("pre_render_skip reason=gate_write error={}", error),
        );
        return ContextGatePrecheckResult::miss(Some(data_version));
    }
    crate::log::info(
        "context-gate",
        &format!(
            "pre_render_suppress host={} key={} reason=data_version hash={} project={}",
            invocation.host.as_env_value(),
            key,
            row.context_hash,
            invocation.project
        ),
    );
    ContextGatePrecheckResult::hit(
        gate_decision(
            String::new(),
            ContextGateAction::Suppressed,
            "suppressed_data_version",
            &key,
            &row.context_hash,
            "suppressed",
        ),
        data_version,
    )
}

fn load_retained_gate_row(
    conn: &rusqlite::Connection,
    host: &str,
    key: &str,
    cutoff_epoch: i64,
) -> Result<Option<GateRow>> {
    conn.query_row(
        "SELECT context_hash, last_emitted_epoch, data_version
         FROM context_injections
         WHERE host = ?1
           AND injection_key = ?2
           AND updated_at_epoch >= ?3",
        rusqlite::params![host, key, cutoff_epoch],
        |row| {
            Ok(GateRow {
                context_hash: row.get(0)?,
                last_emitted_epoch: row.get(1)?,
                data_version: row.get(2)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::policy::ContextLimits;
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

    fn test_request(invocation: &ContextInvocation) -> ContextRequest {
        ContextRequest {
            cwd: invocation.cwd.clone(),
            project: invocation.project.clone(),
            session_id: invocation.session_id.clone(),
            hook_source: invocation.source.clone(),
            current_branch: None,
            host: invocation.host,
            use_colors: invocation.use_colors,
        }
    }

    fn test_policy() -> ContextPolicy {
        ContextPolicy::from_limits(ContextLimits::default())
    }

    fn apply_gate_with_current_data_version(
        conn: &rusqlite::Connection,
        invocation: &ContextInvocation,
        output: String,
    ) -> anyhow::Result<ContextGateDecision> {
        let request = test_request(invocation);
        let policy = test_policy();
        let data_version =
            data_version_hint::compute_data_version_hint(conn, &request, invocation, &policy)?;
        Ok(apply_context_gate_with_data_version(
            conn,
            invocation,
            output,
            Some(&data_version),
        ))
    }

    fn pre_render_for_test(
        conn: &rusqlite::Connection,
        invocation: &ContextInvocation,
    ) -> ContextGatePrecheckResult {
        let request = test_request(invocation);
        let policy = test_policy();
        pre_render_context_gate(conn, invocation, &request, &policy, false)
    }

    #[test]
    fn strict_pre_render_suppresses_existing_session_without_render_output() -> anyhow::Result<()> {
        let _data_dir = crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render");
        let mut invocation = gate_invocation(Some("sess-pre-render"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let result = pre_render_for_test(&conn, &invocation);
        let Some(decision) = result.decision else {
            anyhow::bail!("existing strict gate row should suppress before render");
        };
        assert_eq!(result.precheck, ContextGatePrecheck::Hit);
        assert_eq!(decision.action, ContextGateAction::Suppressed);
        assert_eq!(decision.reason, "suppressed_data_version");
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

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_none());
        Ok(())
    }

    #[test]
    fn strict_pre_render_does_not_suppress_force_or_clear() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-clear");
        let mut invocation = gate_invocation(Some("sess-pre-render-clear"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        invocation.force = true;
        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Off);

        invocation.force = false;
        invocation.source = Some("clear".to_string());
        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Off);
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

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
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

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        Ok(())
    }

    #[test]
    fn strict_pre_render_respects_fallback_cooldown_expiry() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-fallback");
        let _cooldown = ScopedEnvVar::set("REMEM_CONTEXT_GATE_FALLBACK_COOLDOWN_SECS", "900");
        let mut invocation = gate_invocation(None);
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let old_last_emitted = chrono::Utc::now().timestamp().saturating_sub(901);
        conn.execute(
            "UPDATE context_injections
             SET last_emitted_epoch = ?1
             WHERE host = ?2 AND injection_key = ?3",
            params![
                old_last_emitted,
                invocation.host.as_env_value(),
                injection_key(&invocation)
            ],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        Ok(())
    }

    #[test]
    fn strict_pre_render_misses_after_project_memory_write() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-data-change");
        let mut invocation = gate_invocation(Some("sess-pre-render-data-change"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        crate::memory::insert_memory_full(
            &conn,
            Some("sess-pre-render-data-change"),
            &invocation.project,
            Some("context-gate-data-change"),
            "New context memory",
            "A relevant project memory changed.",
            "decision",
            None,
            None,
            "project",
            None,
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_some());
        Ok(())
    }

    #[test]
    fn strict_pre_render_misses_after_same_second_memory_content_change() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-memory-row");
        let mut invocation = gate_invocation(Some("sess-pre-render-memory-row"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;
        let memory_id = crate::memory::insert_memory_full(
            &conn,
            Some("sess-pre-render-memory-row"),
            &invocation.project,
            Some("context-gate-memory-row"),
            "Context gate row hash",
            "Original render-relevant memory text.",
            "decision",
            None,
            None,
            "project",
            None,
        )?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        conn.execute(
            "UPDATE memories
             SET content = 'Changed render-relevant memory text.'
             WHERE id = ?1",
            params![memory_id],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_some());
        Ok(())
    }

    #[test]
    fn strict_pre_render_misses_after_git_commit_signal_change() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-commit");
        let mut invocation = gate_invocation(Some("sess-pre-render-commit"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        conn.execute(
            "INSERT INTO git_commits
             (project, repo_path, sha, short_sha, branch, message,
              authored_at_epoch, changed_files, created_at_epoch, updated_at_epoch)
             VALUES (?1, ?1, 'sha-pre-render-commit', 'sha-pre', NULL,
                     'Change render-driving commit signal', 2000, '[]', 2000, 2000)",
            params![invocation.project],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_some());
        Ok(())
    }

    #[test]
    fn strict_pre_render_misses_after_memory_fact_change() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-fact");
        let mut invocation = gate_invocation(Some("sess-pre-render-fact"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;
        let memory_id = crate::memory::insert_memory_full(
            &conn,
            Some("sess-pre-render-fact"),
            &invocation.project,
            Some("context-gate-fact"),
            "Context gate fact row",
            "A memory with temporal fact labels.",
            "decision",
            None,
            None,
            "project",
            None,
        )?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        conn.execute(
            "INSERT INTO memory_facts
             (project, subject, predicate, object, valid_from_epoch, valid_to_epoch,
              learned_at_epoch, source_memory_id, source_observation_id, source_event_ids,
              confidence, supersedes_fact_id, status, invalidated_at_epoch,
              created_at_epoch, updated_at_epoch)
             VALUES (?1, 'context gate', 'uses_file', 'src/context/render.rs',
                     NULL, NULL, 2000, ?2, NULL, '[]', 0.900000, NULL, 'active',
                     NULL, 2000, 2000)",
            params![invocation.project, memory_id],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_some());
        Ok(())
    }

    #[test]
    fn strict_pre_render_misses_after_unknown_memory_type_content_change() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-custom-row");
        let mut invocation = gate_invocation(Some("sess-pre-render-custom-row"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;
        let memory_id = crate::memory::insert_memory_full(
            &conn,
            Some("sess-pre-render-custom-row"),
            &invocation.project,
            Some("custom/context-gate-row"),
            "Custom context row",
            "Original custom row text.",
            "custom_context",
            None,
            None,
            "project",
            None,
        )?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        conn.execute(
            "UPDATE memories
             SET content = 'Changed custom row text.'
             WHERE id = ?1",
            params![memory_id],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_some());
        Ok(())
    }

    #[test]
    fn strict_pre_render_misses_after_preference_content_change() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-pref-row");
        let mut invocation = gate_invocation(Some("sess-pre-render-pref-row"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;
        let memory_id = crate::memory::insert_memory_full(
            &conn,
            Some("sess-pre-render-pref-row"),
            &invocation.project,
            Some("preference/context-gate-row"),
            "Preference: context gate",
            "Original preference text.",
            "preference",
            None,
            None,
            "project",
            None,
        )?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        conn.execute(
            "UPDATE memories
             SET content = 'Changed preference text.'
             WHERE id = ?1",
            params![memory_id],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_some());
        Ok(())
    }

    #[test]
    fn strict_pre_render_misses_after_workstream_next_action_change() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-workstream");
        let mut invocation = gate_invocation(Some("sess-pre-render-workstream"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;
        conn.execute(
            "INSERT INTO workstreams
             (project, title, status, next_action, created_at_epoch, updated_at_epoch,
              source_project, target_project, owner_scope, owner_key, context_class)
             VALUES (?1, 'Context gate workstream', 'active', 'Original next action',
                     ?2, ?2, ?1, ?1, 'repo', ?1, 'startup_core')",
            params![invocation.project, chrono::Utc::now().timestamp()],
        )?;
        let workstream_id = conn.last_insert_rowid();

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        conn.execute(
            "UPDATE workstreams
             SET next_action = 'Changed next action'
             WHERE id = ?1",
            params![workstream_id],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_some());
        Ok(())
    }

    #[test]
    fn strict_pre_render_ignores_workstream_past_render_limit() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-work-limit");
        let mut invocation = gate_invocation(Some("sess-pre-render-work-limit"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;
        for idx in 0..6 {
            conn.execute(
                "INSERT INTO workstreams
                 (project, title, status, next_action, created_at_epoch, updated_at_epoch,
                  source_project, target_project, owner_scope, owner_key, context_class)
                 VALUES (?1, ?2, 'active', ?3, ?4, ?4, ?1, ?1, 'repo', ?1, 'startup_core')",
                params![
                    invocation.project,
                    format!("Context gate workstream {idx}"),
                    format!("Next action {idx}"),
                    2_000_i64 - idx,
                ],
            )?;
        }

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        conn.execute(
            "UPDATE workstreams
             SET next_action = 'Changed offscreen next action'
             WHERE title = 'Context gate workstream 5'",
            [],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        let Some(decision) = result.decision else {
            anyhow::bail!("off-limit workstream should not invalidate rendered context");
        };
        assert_eq!(result.precheck, ContextGatePrecheck::Hit);
        assert_eq!(decision.reason, "suppressed_data_version");
        Ok(())
    }

    #[test]
    fn strict_pre_render_falls_open_when_data_version_query_fails() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-fail-open");
        let mut invocation = gate_invocation(Some("sess-pre-render-fail-open"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);
        conn.execute("DROP TABLE memories", [])?;

        let result = pre_render_for_test(&conn, &invocation);
        assert!(result.decision.is_none());
        assert_eq!(result.precheck, ContextGatePrecheck::Miss);
        assert!(result.data_version.is_none());
        Ok(())
    }

    #[test]
    fn strict_pre_render_records_suppression_refresh() -> anyhow::Result<()> {
        let _data_dir =
            crate::db::test_support::ScopedTestDataDir::new("context-gate-pre-render-refresh");
        let mut invocation = gate_invocation(Some("sess-pre-render-refresh"));
        invocation.gate_mode = Some("strict".to_string());
        let conn = crate::db::test_support::runtime_connection()?;

        let first = apply_gate_with_current_data_version(
            &conn,
            &invocation,
            "# [/tmp/remem] context now\nBody A\n".to_string(),
        )?;
        assert_eq!(first.action, ContextGateAction::EmittedFull);

        let old_epoch = chrono::Utc::now().timestamp().saturating_sub(60);
        conn.execute(
            "UPDATE context_injections
             SET updated_at_epoch = ?1, suppress_count = 0
             WHERE host = ?2 AND injection_key = ?3",
            params![
                old_epoch,
                invocation.host.as_env_value(),
                injection_key(&invocation)
            ],
        )?;

        let result = pre_render_for_test(&conn, &invocation);
        let Some(decision) = result.decision else {
            anyhow::bail!("existing strict gate row should suppress before render");
        };
        assert_eq!(result.precheck, ContextGatePrecheck::Hit);
        assert_eq!(decision.action, ContextGateAction::Suppressed);

        let (updated_at_epoch, suppress_count): (i64, i64) = conn.query_row(
            "SELECT updated_at_epoch, suppress_count
             FROM context_injections
             WHERE host = ?1 AND injection_key = ?2",
            params![invocation.host.as_env_value(), injection_key(&invocation)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert!(updated_at_epoch > old_epoch);
        assert_eq!(suppress_count, 1);
        Ok(())
    }
}
