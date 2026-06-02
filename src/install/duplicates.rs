use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallPathCandidate {
    pub(crate) path: PathBuf,
    pub(crate) resolved_path: PathBuf,
    pub(crate) version: Option<String>,
    pub(crate) first_on_path: bool,
    pub(crate) configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallPathReport {
    pub(crate) configured_path: Option<PathBuf>,
    pub(crate) configured_resolved_path: Option<PathBuf>,
    pub(crate) candidates: Vec<InstallPathCandidate>,
}

impl InstallPathReport {
    pub(crate) fn has_warning(&self) -> bool {
        self.has_duplicates() || self.first_path_differs_from_configured()
    }

    fn has_duplicates(&self) -> bool {
        self.candidates.len() > 1
    }

    fn first_path_differs_from_configured(&self) -> bool {
        let Some(configured) = self.configured_resolved_path.as_ref() else {
            return false;
        };
        self.candidates
            .first()
            .is_some_and(|candidate| &candidate.resolved_path != configured)
    }
}

pub(crate) fn inspect_install_paths(configured_path: Option<&Path>) -> InstallPathReport {
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path_env).collect();
    collect_install_paths(
        dirs,
        configured_path,
        default_candidate_names(),
        probe_version,
    )
}

pub(crate) fn format_doctor_detail(report: &InstallPathReport) -> String {
    let configured = report
        .configured_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    if report.candidates.is_empty() {
        return format!("no remem executable found on PATH; configured {configured}");
    }

    let candidate_list = report
        .candidates
        .iter()
        .map(format_candidate)
        .collect::<Vec<_>>()
        .join("; ");

    if report.has_warning() {
        return format!(
            "{} remem executable(s) found; configured {}; candidates: {}; fix: remove or upgrade stale installs, or put the intended path first in PATH",
            report.candidates.len(),
            configured,
            candidate_list
        );
    }

    format!(
        "one remem executable found; configured {}; candidate: {}",
        configured, candidate_list
    )
}

pub(crate) fn format_warning_lines(report: &InstallPathReport) -> Vec<String> {
    if !report.has_warning() {
        return Vec::new();
    }

    let mut lines = vec!["Install paths: multiple or mismatched remem commands found".to_string()];
    if let Some(configured) = report.configured_path.as_ref() {
        lines.push(format!("  config -> {}", configured.display()));
    }
    for candidate in &report.candidates {
        let label = if candidate.first_on_path {
            "active"
        } else {
            "stale "
        };
        lines.push(format!("  {label} -> {}", format_candidate(candidate)));
    }
    lines.push(
        "  fix    -> remove or upgrade stale package-manager/manual installs, or put the intended path first in PATH"
            .to_string(),
    );
    lines
}

fn format_candidate(candidate: &InstallPathCandidate) -> String {
    let version = candidate
        .version
        .as_deref()
        .unwrap_or("version unavailable");
    let configured = if candidate.configured {
        ", configured"
    } else {
        ""
    };
    format!("{} ({version}{configured})", candidate.path.display())
}

fn collect_install_paths<F>(
    dirs: Vec<PathBuf>,
    configured_path: Option<&Path>,
    candidate_names: &[&str],
    mut version_probe: F,
) -> InstallPathReport
where
    F: FnMut(&Path) -> Option<String>,
{
    let configured_path = configured_path.map(Path::to_path_buf);
    let configured_resolved_path = configured_path
        .as_deref()
        .map(|path| resolve_configured_path(path, &dirs, candidate_names));
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();

    for dir in dirs {
        for name in candidate_names {
            let candidate_path = dir.join(name);
            if !is_candidate_file(&candidate_path) {
                continue;
            }

            let resolved_path = canonical_or_original(&candidate_path);
            if !seen.insert(resolved_path.clone()) {
                continue;
            }

            let configured = configured_resolved_path
                .as_ref()
                .is_some_and(|path| path == &resolved_path);
            let first_on_path = candidates.is_empty();
            let version = version_probe(&candidate_path);
            candidates.push(InstallPathCandidate {
                path: candidate_path,
                resolved_path,
                version,
                first_on_path,
                configured,
            });
        }
    }

    InstallPathReport {
        configured_path,
        configured_resolved_path,
        candidates,
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn resolve_configured_path(path: &Path, dirs: &[PathBuf], candidate_names: &[&str]) -> PathBuf {
    resolve_bare_command_path(path, dirs, candidate_names)
        .unwrap_or_else(|| canonical_or_original(path))
}

fn resolve_bare_command_path(
    path: &Path,
    dirs: &[PathBuf],
    candidate_names: &[&str],
) -> Option<PathBuf> {
    let command = bare_command_name(path)?;
    for dir in dirs {
        let exact = dir.join(path);
        if is_candidate_file(&exact) {
            return Some(canonical_or_original(&exact));
        }

        for name in candidate_names {
            let candidate = Path::new(name);
            if candidate.file_stem() != Some(command) {
                continue;
            }

            let candidate_path = dir.join(candidate);
            if is_candidate_file(&candidate_path) {
                return Some(canonical_or_original(&candidate_path));
            }
        }
    }
    None
}

fn bare_command_name(path: &Path) -> Option<&OsStr> {
    let mut components = path.components();
    let Some(Component::Normal(name)) = components.next() else {
        return None;
    };
    if components.next().is_some() {
        return None;
    }
    Some(name)
}

fn default_candidate_names() -> &'static [&'static str] {
    candidate_names_for_platform(cfg!(windows))
}

fn candidate_names_for_platform(windows: bool) -> &'static [&'static str] {
    if windows {
        &["remem.exe", "remem.cmd", "remem.bat"]
    } else {
        &["remem"]
    }
}

fn is_candidate_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    is_executable(&metadata)
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    true
}

fn probe_version(path: &Path) -> Option<String> {
    let mut child = Command::new(path)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_millis(500);

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().ok()?;
                return parse_version_output(output.stdout, output.stderr);
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn parse_version_output(stdout: Vec<u8>, stderr: Vec<u8>) -> Option<String> {
    let text = if stdout.is_empty() { stderr } else { stdout };
    let first_line = String::from_utf8_lossy(&text)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if first_line.is_empty() {
        None
    } else {
        Some(first_line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    fn temp_dir(label: &str) -> PathBuf {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "remem-install-paths-{label}-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn write_candidate(dir: &Path, name: &str, version: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, version).expect("write candidate");
        #[cfg(unix)]
        {
            let mut permissions = std::fs::metadata(&path)
                .expect("candidate metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&path, permissions).expect("mark executable");
        }
        path
    }

    #[test]
    fn detects_multiple_unique_unix_candidates() {
        let first = temp_dir("first");
        let second = temp_dir("second");
        let first_path = write_candidate(&first, "remem", "remem 0.4.5");
        let second_path = write_candidate(&second, "remem", "remem 0.4.1");

        let report =
            collect_install_paths(vec![first, second], Some(&first_path), &["remem"], |path| {
                Some(std::fs::read_to_string(path).expect("candidate version"))
            });

        assert!(report.has_warning());
        assert_eq!(report.candidates.len(), 2);
        assert!(report.candidates[0].first_on_path);
        assert!(report.candidates[0].configured);
        assert_eq!(report.candidates[1].path, second_path);
        assert!(format_doctor_detail(&report).contains("2 remem executable(s) found"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_to_same_binary_is_not_duplicate() {
        let real_dir = temp_dir("real");
        let link_dir = temp_dir("link");
        let real = write_candidate(&real_dir, "remem", "remem 0.4.5");
        let link = link_dir.join("remem");
        create_symlink(&real, &link).expect("create symlink");

        let report =
            collect_install_paths(vec![real_dir, link_dir], Some(&real), &["remem"], |path| {
                Some(std::fs::read_to_string(path).unwrap_or_else(|_| path.display().to_string()))
            });

        assert!(!report.has_warning());
        assert_eq!(report.candidates.len(), 1);
    }

    #[test]
    fn warns_when_first_path_differs_from_configured_binary() {
        let stale_dir = temp_dir("stale");
        let configured_dir = temp_dir("configured");
        write_candidate(&stale_dir, "remem", "remem 0.4.1");
        let configured = write_candidate(&configured_dir, "remem", "remem 0.4.5");

        let report = collect_install_paths(
            vec![stale_dir, configured_dir],
            Some(&configured),
            &["remem"],
            |path| Some(std::fs::read_to_string(path).expect("candidate version")),
        );

        assert!(report.has_warning());
        assert!(report.first_path_differs_from_configured());
        let lines = format_warning_lines(&report).join("\n");
        assert!(lines.contains("active ->"));
        assert!(lines.contains("stale  ->"));
    }

    #[test]
    fn configured_command_name_resolves_through_path() {
        let bin_dir = temp_dir("command");
        let configured = PathBuf::from("remem");
        let expected = write_candidate(&bin_dir, "remem", "remem 0.4.5");

        let report = collect_install_paths(vec![bin_dir], Some(&configured), &["remem"], |path| {
            match std::fs::read_to_string(path) {
                Ok(version) => Some(version),
                Err(error) => panic!("candidate version {}: {error}", path.display()),
            }
        });

        assert!(!report.has_warning());
        assert_eq!(report.configured_path, Some(configured));
        assert_eq!(
            report.configured_resolved_path,
            Some(canonical_or_original(&expected))
        );
        assert!(report.candidates[0].configured);
    }

    #[test]
    fn configured_command_name_resolves_platform_wrapper() {
        let bin_dir = temp_dir("wrapper");
        let configured = PathBuf::from("remem");
        let expected = write_candidate(&bin_dir, "remem.exe", "remem 0.4.5");

        let report = collect_install_paths(
            vec![bin_dir],
            Some(&configured),
            &["remem.exe", "remem.cmd", "remem.bat"],
            |path| match std::fs::read_to_string(path) {
                Ok(version) => Some(version),
                Err(error) => panic!("candidate version {}: {error}", path.display()),
            },
        );

        assert!(!report.has_warning());
        assert_eq!(
            report.configured_resolved_path,
            Some(canonical_or_original(&expected))
        );
        assert!(report.candidates[0].configured);
    }

    #[test]
    fn windows_candidate_names_include_wrappers() {
        assert_eq!(
            candidate_names_for_platform(true),
            ["remem.exe", "remem.cmd", "remem.bat"].as_slice()
        );
    }

    #[test]
    fn parses_first_version_line_from_stdout_or_stderr() {
        assert_eq!(
            parse_version_output(b"remem 0.4.5\nextra".to_vec(), Vec::new()),
            Some("remem 0.4.5".to_string())
        );
        assert_eq!(
            parse_version_output(Vec::new(), b"remem 0.4.6\n".to_vec()),
            Some("remem 0.4.6".to_string())
        );
        assert_eq!(parse_version_output(Vec::new(), Vec::new()), None);
    }

    #[cfg(unix)]
    fn create_symlink(from: &Path, to: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(from, to)
    }
}
