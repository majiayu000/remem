use super::types::{Check, Status};

pub(super) fn check_runtime_config() -> Check {
    let path = crate::runtime_config::config_path();
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
        Ok(profile) => Check::new(
            "Runtime config",
            Status::Ok,
            format!(
                "{} default profile={} executor={:?} model={}",
                path.display(),
                profile.profile_name,
                profile.executor,
                profile.model.as_deref().unwrap_or("auto")
            ),
        ),
        Err(error) => Check::new(
            "Runtime config",
            Status::Fail,
            format!("{} invalid: {}", path.display(), error),
        ),
    }
}
