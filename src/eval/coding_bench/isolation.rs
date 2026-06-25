use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

#[derive(Debug)]
pub struct CodexIsolation {
    pub program: String,
    pub args_prefix: Vec<String>,
    pub env: Vec<(String, String)>,
    private_root: PathBuf,
}

impl CodexIsolation {
    pub fn cleanup(&self) {
        if let Err(err) = fs::remove_dir_all(&self.private_root) {
            eprintln!(
                "[coding-bench] failed to remove isolated Codex home {}: {err}",
                self.private_root.display()
            );
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn prepare_codex_isolation(_run_root: &Path, _codex_bin: &str) -> Result<CodexIsolation> {
    bail!("isolated codex benchmark runner requires a host-read sandbox on this platform")
}

#[cfg(target_os = "macos")]
pub fn prepare_codex_isolation(run_root: &Path, codex_bin: &str) -> Result<CodexIsolation> {
    let codex_executable = resolve_executable(codex_bin)?;
    let node_executable = resolve_executable("node").ok();
    let private_root = sibling_private_root(run_root);
    let isolated_home = private_root.join("home");
    let isolated_codex_home = private_root.join("codex-home");
    fs::create_dir_all(&isolated_home)
        .with_context(|| format!("create isolated HOME {}", isolated_home.display()))?;
    fs::create_dir_all(&isolated_codex_home).with_context(|| {
        format!(
            "create isolated CODEX_HOME {}",
            isolated_codex_home.display()
        )
    })?;
    copy_codex_auth(&isolated_codex_home)?;
    let host_home = host_home_dir()?;
    let auth_path = isolated_codex_home.join("auth.json");
    let profile =
        macos_host_read_sandbox_profile(run_root, &private_root, &host_home, &codex_executable);

    Ok(CodexIsolation {
        program: "/bin/sh".to_string(),
        args_prefix: vec![
            "-c".to_string(),
            MACOS_CODEX_SANDBOX_WRAPPER.to_string(),
            "remem-codex-sandbox".to_string(),
            profile,
            codex_executable.to_string_lossy().to_string(),
            auth_path.to_string_lossy().to_string(),
        ],
        env: vec![
            (
                "HOME".to_string(),
                isolated_home.to_string_lossy().to_string(),
            ),
            (
                "CODEX_HOME".to_string(),
                isolated_codex_home.to_string_lossy().to_string(),
            ),
            (
                "PATH".to_string(),
                clean_runner_path(&[codex_executable.as_path()], node_executable.as_deref()),
            ),
        ],
        private_root,
    })
}

pub fn runner_isolation_violation(stdout: &str, stderr: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let allowed_local = format!("{home}/.local");
    let combined = format!("{stdout}\n{stderr}");
    let redacted = combined.replace(&allowed_local, "");
    if redacted.contains(&home) {
        return Some(format!("runner attempted host path access under {home}"));
    }
    [
        "CODEX_HOME",
        "auth.json",
        "codex-home",
        "codex-private",
        "memories_1.sqlite",
        "goals_1.sqlite",
    ]
    .iter()
    .any(|marker| combined.contains(marker))
    .then(|| "runner attempted benchmark-private Codex home access".to_string())
}

fn copy_codex_auth(isolated_codex_home: &Path) -> Result<()> {
    let host_codex_home = host_codex_home_dir()?;
    let auth_src = host_codex_home.join("auth.json");
    let auth_dst = isolated_codex_home.join("auth.json");
    if !auth_src.exists() {
        bail!(
            "codex runner requires host auth at {} to populate isolated CODEX_HOME",
            auth_src.display()
        );
    }
    fs::copy(&auth_src, &auth_dst).with_context(|| {
        format!(
            "copy codex auth from {} to {}",
            auth_src.display(),
            auth_dst.display()
        )
    })?;
    Ok(())
}

fn host_codex_home_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        return Ok(PathBuf::from(path));
    }
    Ok(host_home_dir()?.join(".codex"))
}

fn host_home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME must be set to build an isolated Codex benchmark environment")
}

fn sibling_private_root(run_root: &Path) -> PathBuf {
    let name = run_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("remem-coding-bench-run");
    let private_name = format!("{name}-codex-private");
    run_root
        .parent()
        .map(|parent| parent.join(&private_name))
        .unwrap_or_else(|| run_root.join(&private_name))
}

fn resolve_executable(program: &str) -> Result<PathBuf> {
    let candidate = PathBuf::from(program);
    if candidate.components().count() > 1 {
        return Ok(candidate);
    }
    let path = std::env::var_os("PATH").context("PATH must be set to resolve codex runner")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("could not resolve codex runner executable {program:?} on PATH")
}

fn clean_runner_path(required_executables: &[&Path], optional_node: Option<&Path>) -> String {
    let mut entries = Vec::new();
    for executable in required_executables.iter().copied().chain(optional_node) {
        if let Some(parent) = executable.parent() {
            entries.push(parent.to_string_lossy().to_string());
        }
    }
    entries.extend(
        [
            "/opt/homebrew/opt/python@3.12/libexec/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
        .into_iter()
        .map(str::to_string),
    );
    entries.dedup();
    entries.join(":")
}

#[cfg(target_os = "macos")]
const MACOS_CODEX_SANDBOX_WRAPPER: &str = r#"profile=$1
codex_bin=$2
auth_file=$3
shift 3
sandbox-exec -p "$profile" "$codex_bin" "$@" &
pid=$!
wait "$pid"
status=$?
rm -f "$auth_file"
exit "$status"
"#;

#[cfg(target_os = "macos")]
fn macos_host_read_sandbox_profile(
    run_root: &Path,
    private_root: &Path,
    host_home: &Path,
    codex_executable: &Path,
) -> String {
    let run_root = run_root
        .canonicalize()
        .unwrap_or_else(|_| run_root.to_path_buf());
    let private_root = private_root
        .canonicalize()
        .unwrap_or_else(|_| private_root.to_path_buf());
    let codex_install = host_home.join(".local");
    let codex_executable_parent = codex_executable
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| codex_install.clone());
    format!(
        "(version 1) \
         (allow default) \
         (deny file-read-data (subpath \"{}\")) \
         (allow file-read-data (subpath \"{}\")) \
         (allow file-read-data (subpath \"{}\")) \
         (allow file-read-data (subpath \"{}\")) \
         (allow file-read-data (subpath \"{}\"))",
        escape_profile_path(host_home),
        escape_profile_path(&codex_install),
        escape_profile_path(&codex_executable_parent),
        escape_profile_path(&private_root),
        escape_profile_path(&run_root)
    )
}

#[cfg(target_os = "macos")]
fn escape_profile_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_profile_denies_host_home_but_allows_codex_install_and_run_root() {
        let profile = macos_host_read_sandbox_profile(
            Path::new("/tmp/remem-coding-bench-run"),
            Path::new("/tmp/remem-coding-bench-run-codex-private"),
            Path::new("/Users/example"),
            Path::new("/Users/example/.local/bin/codex"),
        );

        assert!(profile.contains("(deny file-read-data (subpath \"/Users/example\"))"));
        assert!(profile.contains("(allow file-read-data (subpath \"/Users/example/.local\"))"));
        assert!(
            profile.contains("(allow file-read-data (subpath \"/tmp/remem-coding-bench-run\"))")
        );
        assert!(profile.contains(
            "(allow file-read-data (subpath \"/tmp/remem-coding-bench-run-codex-private\"))"
        ));
    }

    #[test]
    fn clean_runner_path_excludes_current_home_virtualenv() {
        let path = clean_runner_path(&[Path::new("/Users/example/.local/bin/codex")], None);
        if let Some(home) = std::env::var_os("HOME") {
            assert!(!path.contains(&home.to_string_lossy().to_string()));
        }
        assert!(path.contains("/usr/bin"));
        assert!(path.contains("/Users/example/.local/bin"));
    }

    #[test]
    fn runner_isolation_violation_ignores_codex_install_path_only() {
        let Some(home) = std::env::var("HOME").ok() else {
            return;
        };
        assert!(runner_isolation_violation(&format!("{home}/.local/bin/codex"), "").is_none());
        assert!(
            runner_isolation_violation(&format!("{home}/.codex/memories/MEMORY.md"), "").is_some()
        );
    }
}
