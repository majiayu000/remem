//! Typed identity for v2 capture / extraction / memory rows.
//!
//! Per SPEC-memory-system-v2.1-revisions §1 M1, the v2 six-tuple
//! `(host, workspace, project, session_id, turn_id, event_id)` is a mix of
//! host-supplied and remem-synthesized fields. This module owns the synthesis
//! rules and the typed wrappers; nothing else should construct these values
//! from raw strings.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};

use crate::git_util::resolve_toplevel;

/// Install-time host. Distinct from the v1 `context::host::HostKind` (which
/// allows `Unknown` for legacy detection): v2.1 M2 forbids `unknown`, and the
/// value is always sourced from the install-baked `--host` argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstallHost {
    ClaudeCode,
    CodexCli,
}

impl InstallHost {
    /// String written into `hosts.name` and matched by capture / extraction
    /// queries. Kept separate from any context/env representations so that a
    /// rename in one layer does not silently widen identity.
    pub fn as_db_value(self) -> &'static str {
        match self {
            InstallHost::ClaudeCode => "claude-code",
            InstallHost::CodexCli => "codex-cli",
        }
    }

    /// Parse from the `--host` CLI argument. v2.1 M2: any other value is an
    /// install error and must be refused at the boundary. `unknown` is
    /// explicitly rejected here, in contrast to `context::host::HostKind`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "claude-code" => Ok(InstallHost::ClaudeCode),
            "codex-cli" => Ok(InstallHost::CodexCli),
            other => Err(anyhow!(
                "invalid host '{other}'; v2 requires --host claude-code or --host codex-cli"
            )),
        }
    }
}

/// Workspace identity. v2.1 M1: synthesized from cwd + `git rev-parse
/// --show-toplevel`, falling back to cwd when the directory is not a git
/// worktree.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceKey {
    pub root_path: PathBuf,
}

impl WorkspaceKey {
    /// Resolve the workspace root for a capture event. Pure: callers pass in
    /// the cwd and the resolved git toplevel (if any). The caller is
    /// responsible for invoking git; this keeps the function unit-testable.
    pub fn from_cwd_and_toplevel(cwd: &Path, git_toplevel: Option<&Path>) -> Self {
        let root_path = git_toplevel.unwrap_or(cwd).to_path_buf();
        Self { root_path }
    }

    /// Convenience wrapper that resolves the git toplevel for `cwd` via the
    /// `git` binary, falling back to `cwd` when the directory is outside any
    /// git worktree or git is unavailable. Spawns one subprocess.
    pub fn from_cwd(cwd: &Path) -> Self {
        let toplevel = resolve_toplevel(cwd);
        Self::from_cwd_and_toplevel(cwd, toplevel.as_deref())
    }
}

/// Project identity within a workspace. Defaults to the workspace root, but
/// may be narrowed via an explicit `--project` label or sub-directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectKey {
    pub workspace: WorkspaceKey,
    pub project_path: PathBuf,
    pub project_key: String,
}

impl ProjectKey {
    pub fn from_workspace(workspace: WorkspaceKey, project_label: Option<&str>) -> Self {
        let project_path = workspace.root_path.clone();
        let project_key = project_label
            .map(str::to_owned)
            .unwrap_or_else(|| project_path.to_string_lossy().into_owned());
        Self {
            workspace,
            project_path,
            project_key,
        }
    }
}

/// Session id, supplied by the host payload (Claude Code & Codex both expose
/// it). Wrapped to avoid mixing it with `event_id` and `turn_id` strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// Turn id. Codex provides it on every turn-scoped hook; Claude Code does not,
/// so remem synthesizes a per-(host, session) monotonic counter (handled in a
/// later Milestone A step against the `sessions` table).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TurnId(pub String);

/// Event id. Always remem-synthesized: neither Claude Code nor Codex exposes
/// a stable per-event id. The composition rule keeps it deterministic so that
/// duplicate hook invocations coalesce on `UNIQUE(host_id, session_id, event_id)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EventId(pub String);

impl EventId {
    /// `event_id = "<turn>:<event_name>" + optional ":<tool_use_id>"`.
    /// `turn` falls back to the literal `no-turn` when the host does not
    /// expose a turn id (Claude Code outside a single user turn).
    pub fn synthesize(turn: Option<&TurnId>, event_name: &str, tool_use_id: Option<&str>) -> Self {
        let turn_part = turn.map(|t| t.0.as_str()).unwrap_or("no-turn");
        let id = match tool_use_id {
            Some(t) => format!("{turn_part}:{event_name}:{t}"),
            None => format!("{turn_part}:{event_name}"),
        };
        EventId(id)
    }
}

/// Full six-tuple captured at hook entry. The capture path passes this
/// straight into `captured_events` after resolving foreign keys for host /
/// workspace / project / session.
#[derive(Debug, Clone)]
pub struct CaptureIdentity {
    pub host: InstallHost,
    pub workspace: WorkspaceKey,
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub event_id: EventId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_host_round_trip() {
        assert_eq!(
            InstallHost::parse("claude-code").unwrap(),
            InstallHost::ClaudeCode
        );
        assert_eq!(
            InstallHost::parse("codex-cli").unwrap(),
            InstallHost::CodexCli
        );
        assert_eq!(InstallHost::ClaudeCode.as_db_value(), "claude-code");
        assert_eq!(InstallHost::CodexCli.as_db_value(), "codex-cli");
    }

    #[test]
    fn install_host_rejects_unknown() {
        let err = InstallHost::parse("unknown").unwrap_err().to_string();
        assert!(err.contains("invalid host"));
        assert!(InstallHost::parse("").is_err());
    }

    #[test]
    fn workspace_prefers_git_toplevel_over_cwd() {
        let cwd = Path::new("/repo/sub/dir");
        let toplevel = Path::new("/repo");
        let ws = WorkspaceKey::from_cwd_and_toplevel(cwd, Some(toplevel));
        assert_eq!(ws.root_path, PathBuf::from("/repo"));
    }

    #[test]
    fn workspace_falls_back_to_cwd_when_not_in_git() {
        let cwd = Path::new("/tmp/scratch");
        let ws = WorkspaceKey::from_cwd_and_toplevel(cwd, None);
        assert_eq!(ws.root_path, PathBuf::from("/tmp/scratch"));
    }

    #[test]
    fn project_defaults_to_workspace_root_path_string() {
        let ws = WorkspaceKey::from_cwd_and_toplevel(Path::new("/repo"), None);
        let project = ProjectKey::from_workspace(ws.clone(), None);
        assert_eq!(project.project_path, PathBuf::from("/repo"));
        assert_eq!(project.project_key, "/repo");
    }

    #[test]
    fn project_uses_explicit_label_when_provided() {
        let ws = WorkspaceKey::from_cwd_and_toplevel(Path::new("/repo"), None);
        let project = ProjectKey::from_workspace(ws, Some("my-project"));
        assert_eq!(project.project_key, "my-project");
    }

    #[test]
    fn event_id_includes_tool_use_id_when_present() {
        let turn = TurnId("t1".into());
        let id = EventId::synthesize(Some(&turn), "PostToolUse", Some("tu_42"));
        assert_eq!(id.0, "t1:PostToolUse:tu_42");
    }

    #[test]
    fn event_id_omits_tool_use_id_for_turn_level_events() {
        let turn = TurnId("t1".into());
        let id = EventId::synthesize(Some(&turn), "UserPromptSubmit", None);
        assert_eq!(id.0, "t1:UserPromptSubmit");
    }

    #[test]
    fn event_id_uses_no_turn_marker_when_host_lacks_turn() {
        let id = EventId::synthesize(None, "SessionStart", None);
        assert_eq!(id.0, "no-turn:SessionStart");
    }
}
