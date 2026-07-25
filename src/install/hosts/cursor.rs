//! Cursor host install surface (GH-824).
//!
//! Unlike Claude/Codex, Cursor is not driven through the two-phase
//! `install_mcp` / `install_hooks` trait methods: both user-level files and
//! the runtime config/receipt are planned together (B-009) and applied
//! through one staged-apply + compensating-rollback coordinator (B-010).
//! `crate::install::runtime` calls these functions directly.

use anyhow::{bail, Result};

use crate::install::cursor_config::plan::{CursorConfigPlan, CursorOperation};
use crate::install::cursor_config::writer::apply_plan;
pub(in crate::install) use crate::install::cursor_config::{
    cursor_detected, cursor_renderer_supported, CURSOR_HOOK_FAILURE_POLICY_LINE,
    CURSOR_SESSION_INIT_LINE,
};

/// Stable non-fatal diagnostic used when `--target auto` skips Cursor on a
/// platform without an approved command renderer (B-001). Never includes
/// configuration content.
pub(in crate::install) const CURSOR_AUTO_SKIP_DIAGNOSTIC: &str =
    "cursor: skipped (no approved hook command renderer on this platform); \
     Claude/Codex hosts continue unaffected";

/// Read-only preflight shared by install/uninstall/dry-run (B-009/B-015).
/// Fails closed on malformed input, schema drift, receipt tampering, and
/// ownership collisions before any store/key/db/token/config side effect.
pub(in crate::install) fn preflight(
    operation: CursorOperation,
    bin: &str,
) -> Result<CursorConfigPlan> {
    if !cursor_renderer_supported() {
        bail!(
            "cursor host is unsupported on this platform (no approved hook command renderer); \
             no host was modified (code=platform_unsupported)"
        );
    }
    CursorConfigPlan::build(operation, bin)
}

/// Re-plans against fresh snapshots and applies through the staged
/// coordinator. Callers must have run `preflight` earlier in the same
/// command; the re-plan revalidates everything against the current on-disk
/// state so cooperative writes (e.g. runtime store/config initialization
/// between preflight and apply) are absorbed while non-cooperative edits
/// still fail closed at the final comparisons.
pub(in crate::install) fn apply(operation: CursorOperation, bin: &str) -> Result<CursorConfigPlan> {
    let plan = preflight(operation, bin)?;
    apply_plan(&plan)?;
    Ok(plan)
}
