use super::types::{Check, Status};

pub(super) fn check_runtime_config() -> Check {
    let path = match crate::runtime_config::config_path() {
        Ok(path) => path,
        Err(error) => return Check::new("Runtime config", Status::Fail, error.to_string()),
    };
    if !path.exists() {
        return Check::new(
            "Runtime config",
            Status::Warn,
            format!("{} not found (run `remem config init`)", path.display()),
        );
    }

    match crate::runtime_config::resolve_memory_ai_profile(
        crate::runtime_config::MemoryAiSelection::default(),
    ) {
        Ok(profile) => {
            if let Err(error) = crate::runtime_config::context_budget_limits() {
                return Check::new(
                    "Runtime config",
                    Status::Fail,
                    format!("{} invalid: {}", path.display(), error),
                );
            }
            if let Err(error) = crate::runtime_config::validate_pricing_config() {
                return Check::new(
                    "Runtime config",
                    Status::Fail,
                    format!("{} invalid: {}", path.display(), error),
                );
            }
            Check::new(
                "Runtime config",
                Status::Ok,
                format!(
                    "{} default profile={} executor={:?} model={}",
                    path.display(),
                    profile.profile_name,
                    profile.executor,
                    profile.model.as_deref().unwrap_or("auto")
                ),
            )
        }
        Err(error) => Check::new(
            "Runtime config",
            Status::Fail,
            format!("{} invalid: {}", path.display(), error),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_context_budget_fails_runtime_config_check() -> anyhow::Result<()> {
        let _guard = crate::runtime_config::TEST_ENV_LOCK.lock()?;
        let path = std::env::temp_dir().join(format!(
            "remem-doctor-context-{}-{}.toml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::write(&path, "[context]\ntotal_char_limit = -1\n")?;
        let previous = std::env::var_os("REMEM_CONFIG");
        unsafe { std::env::set_var("REMEM_CONFIG", &path) };

        let check = check_runtime_config();

        match previous {
            Some(value) => unsafe { std::env::set_var("REMEM_CONFIG", value) },
            None => unsafe { std::env::remove_var("REMEM_CONFIG") },
        }
        std::fs::remove_file(path)?;
        assert_eq!(check.status, Status::Fail);
        assert!(check
            .detail
            .contains("context.total_char_limit must be >= 0"));
        Ok(())
    }
}
