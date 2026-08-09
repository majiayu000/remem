//! Payload-free Context Bundle production capability check (GH-932).

use super::types::{Check, Status};
use crate::context::{
    context_bundle_render_mode, ContextBundleRenderMode, CONTEXT_BUNDLE_RENDER_MODE_ENV,
    SESSIONSTART_RELEVANCE_POLICY_VERSION,
};
use crate::context_bundle::CONTEXT_BUNDLE_SCHEMA_VERSION;
use crate::retrieval_router::{RETRIEVAL_PLAN_SCHEMA_VERSION, RETRIEVAL_ROUTER_POLICY_VERSION};

const CHECK_NAME: &str = "Context compiler";

pub(super) fn check_context_compiler() -> Check {
    match context_bundle_render_mode() {
        Ok(ContextBundleRenderMode::Bundle) => Check::new(
            CHECK_NAME,
            Status::Ok,
            capability_detail("ready", "bundle", "full"),
        ),
        Ok(ContextBundleRenderMode::Legacy) => Check::new(
            CHECK_NAME,
            Status::Warn,
            format!(
                "{}; legacy rollback is active; unset {CONTEXT_BUNDLE_RENDER_MODE_ENV} or set it to bundle to restore the production consumer",
                capability_detail("ready", "legacy", "legacy_rollback")
            ),
        ),
        Err(error) => Check::new(
            CHECK_NAME,
            Status::Fail,
            format!(
                "{} error={error}",
                capability_detail("blocked", "invalid", "blocked")
            ),
        ),
    }
}

fn capability_detail(capability: &str, render_mode: &str, degraded_mode: &str) -> String {
    format!(
        "consumer=session_start capability={capability} render_mode={render_mode} degraded_mode={degraded_mode} \
bundle_schema=v{CONTEXT_BUNDLE_SCHEMA_VERSION} plan_schema=v{RETRIEVAL_PLAN_SCHEMA_VERSION} \
router_policy={RETRIEVAL_ROUTER_POLICY_VERSION} \
relevance_policy={SESSIONSTART_RELEVANCE_POLICY_VERSION} \
payload=omitted plan_debug=`remem context-plan --task <task> --json`"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_render_mode(value: Option<&str>, test: impl FnOnce()) {
        let _guard = crate::runtime_config::TEST_ENV_LOCK
            .lock()
            .expect("environment lock should acquire");
        let previous = std::env::var_os(CONTEXT_BUNDLE_RENDER_MODE_ENV);
        match value {
            Some(value) => unsafe {
                std::env::set_var(CONTEXT_BUNDLE_RENDER_MODE_ENV, value);
            },
            None => unsafe {
                std::env::remove_var(CONTEXT_BUNDLE_RENDER_MODE_ENV);
            },
        }
        test();
        match previous {
            Some(previous) => unsafe {
                std::env::set_var(CONTEXT_BUNDLE_RENDER_MODE_ENV, previous);
            },
            None => unsafe {
                std::env::remove_var(CONTEXT_BUNDLE_RENDER_MODE_ENV);
            },
        }
    }

    #[test]
    fn bundle_mode_reports_payload_free_full_capability() {
        with_render_mode(None, || {
            let check = check_context_compiler();
            assert_eq!(check.status, Status::Ok);
            assert!(check.detail.contains("render_mode=bundle"));
            assert!(check.detail.contains("degraded_mode=full"));
            assert!(check.detail.contains("payload=omitted"));
            assert!(!check.detail.contains("memory:"));
        });
    }

    #[test]
    fn legacy_mode_is_visible_as_degraded_rollback() {
        with_render_mode(Some("legacy"), || {
            let check = check_context_compiler();
            assert_eq!(check.status, Status::Warn);
            assert!(check.detail.contains("render_mode=legacy"));
            assert!(check.detail.contains("degraded_mode=legacy_rollback"));
        });
    }

    #[test]
    fn invalid_mode_fails_with_the_runtime_parser_error() {
        with_render_mode(Some("best-effort"), || {
            let check = check_context_compiler();
            assert_eq!(check.status, Status::Fail);
            assert!(check.detail.contains("render_mode=invalid"));
            assert!(check.detail.contains("must be 'bundle' or 'legacy'"));
        });
    }
}
