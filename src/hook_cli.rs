//! Dedicated hook command surface for `remem-hook` and `remem` dispatch.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::{context, observe, summarize};

pub const HOOK_COMMANDS: &[&str] = &["context", "session-init", "observe", "summarize"];

pub fn hook_subcommand_uses_slim_binary(subcommand: &str) -> bool {
    HOOK_COMMANDS.contains(&subcommand)
}

pub fn is_full_remem_binary(path: &Path) -> bool {
    binary_stem_eq(path, "remem")
}

pub fn is_hook_binary(path: &Path) -> bool {
    binary_stem_eq(path, "remem-hook")
}

fn binary_stem_eq(path: &Path, expected: &str) -> bool {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == expected)
}

fn platform_binary_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

fn sibling_named_path(bin: &Path, stem: &str) -> Option<PathBuf> {
    Some(bin.parent()?.join(platform_binary_name(stem)))
}

fn sibling_named_binary(bin: &Path, stem: &str) -> Option<PathBuf> {
    let candidate = sibling_named_path(bin, stem)?;
    executable_file(&candidate).then_some(candidate)
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub fn sibling_hook_binary(remem_bin: &Path) -> Option<PathBuf> {
    sibling_named_binary(remem_bin, "remem-hook")
}

pub fn sibling_full_binary(hook_bin: &Path) -> Option<PathBuf> {
    sibling_named_binary(hook_bin, "remem")
}

pub fn sibling_full_binary_path(hook_bin: &Path) -> Option<PathBuf> {
    sibling_named_path(hook_bin, "remem")
}

pub fn hook_invocation_binary(remem_bin: &Path, subcommand: &str) -> PathBuf {
    if hook_subcommand_uses_slim_binary(subcommand) {
        if let Some(hook) = sibling_hook_binary(remem_bin) {
            return hook;
        }
    }
    remem_bin.to_path_buf()
}

pub fn hook_executable_is_allowed(
    invocation: &Path,
    expected_remem: &Path,
    subcommand: &str,
) -> bool {
    if invocation == expected_remem {
        return !is_hook_binary(invocation) || executable_file(invocation);
    }
    hook_subcommand_uses_slim_binary(subcommand)
        && sibling_hook_binary(expected_remem).as_deref() == Some(invocation)
}

/// Prefer the full `remem` path when hooks mention both binaries. Infer it
/// from a sibling `remem-hook` when that is the only configured executable.
pub fn preferred_expected_hook_executable<S: AsRef<str>>(paths: &[S]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    if let Some(full) = paths
        .iter()
        .find(|path| is_full_remem_binary(Path::new(path.as_ref())))
    {
        return Some(full.as_ref().to_string());
    }
    for path in paths {
        let hook = Path::new(path.as_ref());
        if is_hook_binary(hook) {
            if let Some(full) = sibling_full_binary(hook) {
                return Some(full.to_string_lossy().into_owned());
            }
        }
    }
    Some(paths[0].as_ref().to_string())
}

#[derive(Parser)]
#[command(
    name = "remem-hook",
    about = "Slim remem host-hook entry (context, session-init, observe, summarize)"
)]
struct HookCli {
    #[command(subcommand)]
    command: HookCommand,
}

#[derive(Subcommand)]
enum HookCommand {
    Context {
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        color: bool,
        #[arg(long)]
        debug: bool,
        #[arg(long)]
        force: bool,
        #[arg(long, value_name = "off|auto|strict|delta")]
        gate: Option<String>,
    },
    SessionInit {
        #[arg(long)]
        host: Option<String>,
    },
    Observe {
        #[arg(long)]
        host: Option<String>,
    },
    Summarize {
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        profile: Option<String>,
    },
}

pub async fn run() -> Result<()> {
    crate::hook_runtime::enter_hook_runtime_mode();
    match HookCli::parse().command {
        HookCommand::Context {
            cwd,
            session_id,
            host,
            color,
            debug,
            force,
            gate,
        } => run_context(cwd, session_id, host, color, debug, force, gate).await,
        HookCommand::SessionInit { host } => run_session_init(host).await,
        HookCommand::Observe { host } => run_observe(host).await,
        HookCommand::Summarize { host, profile } => run_summarize(host, profile).await,
    }
}

pub(crate) async fn run_context(
    cwd: Option<String>,
    session_id: Option<String>,
    host: Option<String>,
    color: bool,
    debug: bool,
    force: bool,
    gate: Option<String>,
) -> Result<()> {
    if remem_hooks_disabled() {
        return Ok(());
    }
    match parse_explicit_hook_host(host.as_deref())? {
        Some(crate::identity::InstallHost::Cursor) => context::generate_cursor_context_from_stdin(),
        _ => context::generate_context_from_cli(cwd, session_id, color, host, debug, force, gate),
    }
}

pub(crate) async fn run_session_init(host: Option<String>) -> Result<()> {
    if remem_hooks_disabled() {
        return Ok(());
    }
    if matches!(
        parse_explicit_hook_host(host.as_deref())?,
        Some(crate::identity::InstallHost::Cursor)
    ) {
        anyhow::bail!(
            "session-init is not supported on --host cursor; \
             Cursor beforeSubmitPrompt is permit/block-only (GH-823 B-006)"
        );
    }
    observe::session_init(host.as_deref()).await
}

pub(crate) async fn run_observe(host: Option<String>) -> Result<()> {
    if remem_hooks_disabled() {
        return Ok(());
    }
    match parse_explicit_hook_host(host.as_deref())? {
        Some(crate::identity::InstallHost::Cursor) => observe::observe_cursor().await,
        _ => observe::observe(host.as_deref()).await,
    }
}

pub(crate) async fn run_summarize(host: Option<String>, profile: Option<String>) -> Result<()> {
    if remem_hooks_disabled() {
        return Ok(());
    }
    match parse_explicit_hook_host(host.as_deref())? {
        Some(crate::identity::InstallHost::Cursor) => summarize::summarize_cursor().await,
        _ => summarize::summarize(host.as_deref(), profile.as_deref()).await,
    }
}

pub(crate) fn parse_explicit_hook_host(
    host: Option<&str>,
) -> Result<Option<crate::identity::InstallHost>> {
    host.map(crate::identity::InstallHost::parse).transpose()
}

pub(crate) fn remem_hooks_disabled() -> bool {
    std::env::var("REMEM_DISABLE_HOOKS")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{HookCli, HookCommand, HOOK_COMMANDS};

    fn make_executable(path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("set executable mode");
        }
    }

    #[test]
    fn hook_command_names_are_stable() {
        assert_eq!(
            HOOK_COMMANDS,
            ["context", "session-init", "observe", "summarize"]
        );
    }

    #[test]
    fn parses_observe_host() {
        let cli = HookCli::try_parse_from(["remem-hook", "observe", "--host", "codex-cli"])
            .expect("observe should parse");
        match cli.command {
            HookCommand::Observe { host } => assert_eq!(host.as_deref(), Some("codex-cli")),
            _ => panic!("expected observe"),
        }
    }

    #[test]
    fn sibling_hook_binary_used_only_for_slim_commands() {
        let dir = std::env::temp_dir().join(format!(
            "remem-hook-sibling-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let remem = dir.join("remem");
        let hook = dir.join("remem-hook");
        std::fs::write(&remem, []).expect("touch remem");
        std::fs::write(&hook, []).expect("touch remem-hook");
        make_executable(&remem);
        make_executable(&hook);

        assert_eq!(
            super::sibling_hook_binary(&remem).as_deref(),
            Some(hook.as_path())
        );
        assert_eq!(
            super::sibling_full_binary(&hook).as_deref(),
            Some(remem.as_path())
        );
        assert_eq!(super::hook_invocation_binary(&remem, "observe"), hook);
        assert_eq!(super::hook_invocation_binary(&remem, "rules"), remem);
        assert!(super::hook_executable_is_allowed(&hook, &remem, "context"));
        assert!(!super::hook_executable_is_allowed(&hook, &remem, "rules"));
        assert_eq!(
            super::preferred_expected_hook_executable(&[
                hook.to_string_lossy().into_owned(),
                remem.to_string_lossy().into_owned(),
            ])
            .as_deref(),
            remem.to_str()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_sibling_hook_is_not_selected_or_allowed() {
        let dir = std::env::temp_dir().join(format!(
            "remem-hook-non-executable-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let remem = dir.join("remem");
        let hook = dir.join("remem-hook");
        std::fs::write(&remem, []).expect("touch remem");
        std::fs::write(&hook, []).expect("touch remem-hook");
        make_executable(&remem);

        assert_eq!(super::sibling_hook_binary(&remem), None);
        assert_eq!(super::hook_invocation_binary(&remem, "observe"), remem);
        assert!(!super::hook_executable_is_allowed(&hook, &hook, "observe"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_full_binary_commands() {
        for args in [
            ["remem-hook", "worker"].as_slice(),
            ["remem-hook", "eval"].as_slice(),
            ["remem-hook", "mcp"].as_slice(),
        ] {
            assert!(
                HookCli::try_parse_from(args).is_err(),
                "expected rejection for {args:?}"
            );
        }
    }
}
