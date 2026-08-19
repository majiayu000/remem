use super::*;

fn with_config_path<T>(path: &std::path::Path, f: impl FnOnce() -> T) -> T {
    let _guard = super::super::TEST_ENV_LOCK
        .lock()
        .expect("env lock should acquire");
    let old = std::env::var("REMEM_CONFIG").ok();
    unsafe { std::env::set_var("REMEM_CONFIG", path) };
    let result = f();
    match old {
        Some(value) => unsafe { std::env::set_var("REMEM_CONFIG", value) },
        None => unsafe { std::env::remove_var("REMEM_CONFIG") },
    }
    result
}

fn temp_config_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "remem-{label}-{}-{}.toml",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ))
}

#[test]
fn default_config_contains_context_budget_defaults() {
    let text = super::super::default_config_text();
    let defaults = ContextLimits::default();
    assert!(text.contains("[context]"), "{text}");
    assert!(
        text.contains(&format!("total_char_limit = {}", defaults.total_char_limit)),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "preference_global_limit = {}",
            defaults.preference_global_limit
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "relevance_k = {}",
            defaults.sessionstart_relevance_k
        )),
        "{text}"
    );
}

#[test]
fn missing_section_uses_compiled_defaults() -> Result<()> {
    let path = temp_config_path("context-budget-missing");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(&path, "version = 1\n")?;
        assert_eq!(context_budget_limits()?, ContextLimits::default());
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn context_section_overrides_selected_budgets() -> Result<()> {
    let path = temp_config_path("context-budget-override");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(
            &path,
            "[context]\ntotal_char_limit = 8000\npreference_global_limit = 0\nrelevance_k = 0\n",
        )?;
        let limits = context_budget_limits()?;
        assert_eq!(limits.total_char_limit, 8_000);
        assert_eq!(limits.preference_global_limit, 0);
        assert_eq!(limits.sessionstart_relevance_k, 0);
        assert_eq!(
            limits.core_char_limit,
            ContextLimits::default().core_char_limit
        );
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn context_section_rejects_non_integer_and_negative() -> Result<()> {
    for (label, body) in [
        ("string", "[context]\ntotal_char_limit = \"8000\"\n"),
        ("negative", "[context]\ncore_item_limit = -1\n"),
    ] {
        let path = temp_config_path(&format!("context-budget-{label}"));
        with_config_path(&path, || -> Result<()> {
            std::fs::write(&path, body)?;
            let err = context_budget_limits().expect_err("invalid context budget must fail");
            assert!(err.to_string().contains("context."), "{err}");
            Ok(())
        })?;
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn env_override_wins_over_context_section() -> Result<()> {
    let path = temp_config_path("context-budget-env-wins");
    with_config_path(&path, || -> Result<()> {
        std::fs::write(&path, "[context]\ntotal_char_limit = 8000\n")?;
        let previous = std::env::var_os("REMEM_CONTEXT_TOTAL_CHAR_LIMIT");
        unsafe { std::env::set_var("REMEM_CONTEXT_TOTAL_CHAR_LIMIT", "7000") };
        let limits = crate::context::ContextLimits::from_runtime();
        match previous {
            Some(value) => unsafe { std::env::set_var("REMEM_CONTEXT_TOTAL_CHAR_LIMIT", value) },
            None => unsafe { std::env::remove_var("REMEM_CONTEXT_TOTAL_CHAR_LIMIT") },
        }
        assert_eq!(limits?.total_char_limit, 7_000);
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn context_budget_can_be_set_through_config_cli() -> Result<()> {
    let path = temp_config_path("context-budget-cli");
    with_config_path(&path, || -> Result<()> {
        super::super::init_config()?;
        super::super::set_config_value("context.total_char_limit", "9000")?;
        let limits = context_budget_limits()?;
        let text = std::fs::read_to_string(&path)?;
        assert_eq!(limits.total_char_limit, 9_000);
        assert!(text.contains("total_char_limit = 9000"), "{text}");
        Ok(())
    })?;
    std::fs::remove_file(path)?;
    Ok(())
}
