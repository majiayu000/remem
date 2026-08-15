use anyhow::Result;

use super::generate_context_output_for_invocation;
use crate::context::hook_warning::append_hook_integrity_warning;
use crate::context::invocation::{
    direct_context_invocation, resolve_context_invocation, resolve_cursor_context_invocation,
    ContextCliOptions, ContextInvocation,
};

pub fn generate_context(
    cwd: &str,
    session_id: Option<&str>,
    use_colors: bool,
    host_arg: Option<&str>,
    debug: bool,
) -> Result<()> {
    let invocation = direct_context_invocation(cwd, session_id, use_colors, host_arg, debug);
    generate_context_for_invocation(invocation, false)
}

pub fn generate_context_from_cli(
    cwd: Option<String>,
    session_id: Option<String>,
    use_colors: bool,
    host: Option<String>,
    debug: bool,
    force: bool,
    gate_mode: Option<String>,
) -> Result<()> {
    let (invocation, stdin_warning) = resolve_context_invocation(ContextCliOptions {
        cwd,
        session_id,
        host,
        use_colors,
        debug,
        force,
        gate_mode,
    })?;
    let mut stdout = generate_context_output_for_invocation(invocation, true)?;
    append_hook_integrity_warning(&mut stdout, stdin_warning.as_deref());
    print!("{stdout}");
    Ok(())
}

/// Cursor `remem context` entrypoint (GH-823): bounded stdin read, strict
/// exact `sessionStart` validation, then the shared render pipeline. Any
/// parse/limit failure returns before context generation with empty stdout
/// and no side effects (B-009); no CLI/current-cwd fallback exists.
pub fn generate_cursor_context_from_stdin() -> Result<()> {
    let bytes = crate::cursor_hook::input::read_bounded_hook_stdin(&mut std::io::stdin().lock())?;
    generate_cursor_context_from_bytes(&bytes)
}

pub fn generate_cursor_context_from_bytes(bytes: &[u8]) -> Result<()> {
    let event = crate::cursor_hook::input::parse_session_start(bytes)?;
    let invocation = resolve_cursor_context_invocation(&event);
    generate_context_for_invocation(invocation, true)
}

pub(super) fn generate_context_for_invocation(
    invocation: ContextInvocation,
    use_gate: bool,
) -> Result<()> {
    let stdout = generate_context_output_for_invocation(invocation, use_gate)?;
    print!("{stdout}");
    Ok(())
}
