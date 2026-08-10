//! Database-open fallback rendering for SessionStart.

use super::super::format::char_len;
use super::super::policy::ContextPolicy;
use super::super::types::{ContextLoadError, ContextRequest};
use super::{enforce_total_char_limit_preserving_footer, stats, RenderedContext};

pub(super) fn open_context_connection_or_error(
    request: &ContextRequest,
    policy: &ContextPolicy,
) -> std::result::Result<rusqlite::Connection, Box<RenderedContext>> {
    match crate::db::open_db_no_migrate() {
        Ok(conn) => Ok(conn),
        Err(error) => {
            crate::log::error(
                "context",
                &format!("db open failed for project={}: {}", request.project, error),
            );
            Err(Box::new(render_context_open_error(request, policy, error)))
        }
    }
}

fn render_context_open_error(
    request: &ContextRequest,
    policy: &ContextPolicy,
    error: anyhow::Error,
) -> RenderedContext {
    let mut output = super::super::render_error::context_error_output(
        request,
        &[ContextLoadError::new(
            "database",
            format!("failed to open remem database: {error}"),
        )],
    );
    let mut stats = stats::empty_stats(request);
    stats.total_char_limit = policy.limits.total_char_limit;
    stats.output_chars = char_len(&output);
    enforce_total_char_limit_preserving_footer(&mut output, policy.limits.total_char_limit, "");
    RenderedContext {
        output,
        stats,
        audit_items: Vec::new(),
        data_version: None,
        has_load_errors: true,
        context_bundle: None,
    }
}
