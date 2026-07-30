use crate::db;

pub(super) const ADMIN_REQUIRED_ARCHIVED_CANDIDATE_LIMIT: i64 = 5;
const ADMIN_REQUIRED_ARCHIVED_FIELD_DISPLAY_BYTES: usize = 80;

pub(super) fn admin_required_archived_recovery_detail(
    candidates: &[db::pending::admin::AdminRequiredArchivedLegacyPendingRow],
    total: usize,
) -> String {
    let details = candidates
        .iter()
        .map(admin_required_archived_candidate_detail)
        .collect::<Vec<_>>()
        .join("; ");
    format!(
        "admin-required archived candidates (showing {} of {total}, oldest first): {details}",
        candidates.len()
    )
}

fn admin_required_archived_candidate_detail(
    candidate: &db::pending::admin::AdminRequiredArchivedLegacyPendingRow,
) -> String {
    let failure_class = candidate
        .failure_class
        .as_deref()
        .map(bounded_debug_field)
        .unwrap_or_else(|| "<null>".to_string());
    let metadata = format!(
        "candidate id={} host={} failure_class={} archived_at_epoch={}",
        candidate.id,
        bounded_debug_field(&candidate.host),
        failure_class,
        candidate.archived_at_epoch
    );
    if matches!(
        candidate.host.as_str(),
        crate::runtime_config::CLAUDE_HOST | crate::runtime_config::CODEX_HOST
    ) {
        return format!(
            "{metadata}; preview `remem pending recover-archived --id {} --dry-run`; apply `remem pending recover-archived --id {}`",
            candidate.id, candidate.id
        );
    }
    format!(
        "{metadata}; unknown host requires explicit `--host`; preview `remem pending recover-archived --id {} --host claude-code --dry-run`; apply `remem pending recover-archived --id {} --host claude-code`; alternatively preview `remem pending recover-archived --id {} --host codex-cli --dry-run`; apply `remem pending recover-archived --id {} --host codex-cli`",
        candidate.id, candidate.id, candidate.id, candidate.id
    )
}

fn bounded_debug_field(value: &str) -> String {
    let truncated = db::truncate_str(value, ADMIN_REQUIRED_ARCHIVED_FIELD_DISPLAY_BYTES);
    if truncated.len() == value.len() {
        return format!("{truncated:?}");
    }
    let displayed = format!("{truncated}…");
    format!("{displayed:?}")
}
