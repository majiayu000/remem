use super::*;

fn with_migration_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
    let _guard = TEST_ENV_LOCK.lock().expect("env lock should acquire");
    let old = std::env::var("REMEM_CONFIG").ok();
    unsafe { std::env::set_var("REMEM_CONFIG", path) };
    let result = f();
    match old {
        Some(value) => unsafe { std::env::set_var("REMEM_CONFIG", value) },
        None => unsafe { std::env::remove_var("REMEM_CONFIG") },
    }
    result
}

fn migration_temp_config_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{}.toml",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

fn legacy_claude_gate_config() -> &'static str {
    "[memory_ai.hosts.claude-code]\nmemory_profile = \"claude\"\ncontext_gate = \"off\"\n"
}

#[test]
fn migrate_legacy_claude_context_gate_dry_run_does_not_write() -> Result<()> {
    let path = migration_temp_config_path("runtime-claude-gate-migrate-dry-run");
    with_migration_config_path(&path, || -> Result<()> {
        std::fs::write(&path, legacy_claude_gate_config())?;

        let migration = migrate_legacy_claude_context_gate(true)?;
        let text = std::fs::read_to_string(&path)?;
        let host = resolve_host_runtime_config(Some("claude-code"))?;

        assert!(migration.changed);
        assert!(migration.dry_run);
        assert_eq!(migration.old_gate.as_deref(), Some("off"));
        assert_eq!(migration.new_gate.as_deref(), Some("auto"));
        assert_eq!(host.context_gate.as_deref(), Some("off"));
        assert!(text.contains("context_gate = \"off\""), "{text}");
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn migrate_legacy_claude_context_gate_applies_explicit_user_command() -> Result<()> {
    let path = migration_temp_config_path("runtime-claude-gate-migrate-apply");
    with_migration_config_path(&path, || -> Result<()> {
        std::fs::write(&path, legacy_claude_gate_config())?;

        let migration = migrate_legacy_claude_context_gate(false)?;
        let text = std::fs::read_to_string(&path)?;
        let host = resolve_host_runtime_config(Some("claude-code"))?;

        assert!(migration.changed);
        assert!(!migration.dry_run);
        assert_eq!(migration.old_gate.as_deref(), Some("off"));
        assert_eq!(migration.new_gate.as_deref(), Some("auto"));
        assert_eq!(host.context_gate.as_deref(), Some("auto"));
        assert!(text.contains("context_gate = \"auto\""), "{text}");
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn migrate_legacy_claude_context_gate_leaves_current_config_unchanged() -> Result<()> {
    let path = migration_temp_config_path("runtime-claude-gate-migrate-current");
    with_migration_config_path(&path, || -> Result<()> {
        init_config()?;

        let migration = migrate_legacy_claude_context_gate(false)?;
        let host = resolve_host_runtime_config(Some("claude-code"))?;

        assert!(!migration.changed);
        assert_eq!(migration.old_gate.as_deref(), Some("auto"));
        assert_eq!(migration.new_gate.as_deref(), Some("auto"));
        assert_eq!(host.context_gate.as_deref(), Some("auto"));
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}
