use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{
    db,
    memory::procedure::{
        ensure_existing_export_registry_match, load_export_eligible_procedure,
        procedure_export_slug, record_procedure_export, render_procedure_export,
        ProcedureExportFormat, ProcedureExportRecordRequest, ProcedureExportSource,
        PROCEDURE_EXPORT_DRAFT_MARKER,
    },
};

const DEFAULT_DRAFT_DIR: &str = "remem-drafts";
const GENERATED_AT_PREFIX: &str = "- Generated at: `";
const REMEM_VERSION_PREFIX: &str = "- remem version: `";

pub(super) fn run_procedure_export(
    memory_id: i64,
    format: ProcedureExportFormat,
    out_dir: Option<&Path>,
    overwrite_generated: bool,
) -> Result<()> {
    let conn = db::open_db()?;
    let source = load_export_eligible_procedure(&conn, memory_id)?;
    let exported_at_epoch = chrono::Utc::now().timestamp();
    let rendered = render_procedure_export(&source, format, exported_at_epoch)?;
    let cwd = std::env::current_dir().context("resolve current directory for procedure export")?;
    let out_dir = procedure_export_out_dir(out_dir);
    let target = export_target_path(&out_dir, &source, format);
    let invocation = ProcedureExportCliInvocationGuard::current()?;
    verify_existing_target_registry(&conn, &source, format, &target, &cwd, overwrite_generated)?;
    let result = write_procedure_export_draft(ProcedureExportWriteRequest {
        source: &source,
        format,
        out_dir: &out_dir,
        rendered: &rendered,
        overwrite_generated,
        invocation,
    })?;
    if let Err(error) = record_procedure_export(
        &conn,
        ProcedureExportRecordRequest {
            source: &source,
            format,
            output_path: &result.path,
            content: &rendered,
            cwd: &cwd,
            exported_at_epoch,
        },
    ) {
        if let Err(rollback_error) = rollback_unregistered_draft(&result, &rendered) {
            return Err(error).with_context(|| {
                format!(
                    "record procedure export registry row; additionally failed to roll back unregistered draft {}: {rollback_error}",
                    result.path.display()
                )
            });
        }
        return Err(error).with_context(|| {
            format!(
                "record procedure export registry row; rolled back unregistered draft {}",
                result.path.display()
            )
        });
    }

    if result.overwritten {
        println!(
            "Overwrote unchanged remem-generated procedure draft: {}",
            result.path.display()
        );
    } else {
        println!("Wrote procedure draft: {}", result.path.display());
    }
    Ok(())
}

struct ProcedureExportWriteRequest<'a> {
    source: &'a ProcedureExportSource,
    format: ProcedureExportFormat,
    out_dir: &'a Path,
    rendered: &'a str,
    overwrite_generated: bool,
    invocation: ProcedureExportCliInvocationGuard,
}

#[derive(Debug)]
struct ProcedureExportWriteResult {
    path: PathBuf,
    overwritten: bool,
    previous_content: Option<String>,
}

fn write_procedure_export_draft(
    request: ProcedureExportWriteRequest<'_>,
) -> Result<ProcedureExportWriteResult> {
    request.invocation.require_cli_procedure_export()?;
    reject_high_context_path(request.out_dir)?;

    let target = export_target_path(request.out_dir, request.source, request.format);
    reject_high_context_path(&target)?;
    let previous_content =
        ensure_writable_target(&target, request.rendered, request.overwrite_generated)?;

    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("procedure export target has no parent directory"))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create procedure export directory {}", parent.display()))?;
    write_atomically(&target, request.rendered)?;

    Ok(ProcedureExportWriteResult {
        path: target,
        overwritten: previous_content.is_some(),
        previous_content,
    })
}

#[derive(Clone, Copy)]
struct ProcedureExportCliInvocationGuard {
    is_cli_procedure_export: bool,
}

impl ProcedureExportCliInvocationGuard {
    fn current() -> Result<Self> {
        if cli_args_are_procedure_export(std::env::args_os().skip(1)) {
            return Ok(Self {
                is_cli_procedure_export: true,
            });
        }
        bail!(
            "procedure export writer is only available from `remem procedures export`; worker, hook, MCP, and background entrypoints must not write procedure drafts"
        );
    }

    fn require_cli_procedure_export(self) -> Result<()> {
        if self.is_cli_procedure_export {
            return Ok(());
        }
        bail!(
            "procedure export writer is only available from `remem procedures export`; worker, hook, MCP, and background entrypoints must not write procedure drafts"
        );
    }

    #[cfg(test)]
    fn assume_cli_for_test() -> Self {
        Self {
            is_cli_procedure_export: true,
        }
    }

    #[cfg(test)]
    fn background_entrypoint_for_test() -> Self {
        Self {
            is_cli_procedure_export: false,
        }
    }
}

fn cli_args_are_procedure_export<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string_lossy().to_ascii_lowercase());
    matches!(
        (args.next().as_deref(), args.next().as_deref()),
        (Some("procedures"), Some("export"))
    )
}

fn procedure_export_out_dir(out_dir: Option<&Path>) -> PathBuf {
    out_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DRAFT_DIR))
}

fn verify_existing_target_registry(
    conn: &rusqlite::Connection,
    source: &ProcedureExportSource,
    format: ProcedureExportFormat,
    target: &Path,
    cwd: &Path,
    overwrite_generated: bool,
) -> Result<()> {
    if !overwrite_generated || !target.exists() {
        return Ok(());
    }
    let existing = std::fs::read_to_string(target)
        .with_context(|| format!("read existing procedure export {}", target.display()))?;
    ensure_existing_export_registry_match(conn, source, format, target, cwd, &existing)
}

fn export_target_path(
    out_dir: &Path,
    source: &ProcedureExportSource,
    format: ProcedureExportFormat,
) -> PathBuf {
    let slug = procedure_export_slug(source);
    match format {
        ProcedureExportFormat::ClaudeSkill => out_dir.join(slug).join("SKILL.md"),
        ProcedureExportFormat::CodexPrompt => out_dir.join(format!("{slug}.codex-prompt.md")),
        ProcedureExportFormat::RunbookMd => out_dir.join(format!("{slug}.runbook.md")),
    }
}

fn ensure_writable_target(
    target: &Path,
    rendered: &str,
    overwrite_generated: bool,
) -> Result<Option<String>> {
    if !target.exists() {
        return Ok(None);
    }

    let existing = std::fs::read_to_string(target)
        .with_context(|| format!("read existing procedure export {}", target.display()))?;
    if existing != rendered && !same_generated_draft_except_generated_at(&existing, rendered) {
        bail!(
            "procedure export target already exists and may be reviewed or user-edited: {}; choose --out <new-dir> or rename the existing draft",
            target.display()
        );
    }
    if !overwrite_generated {
        bail!(
            "procedure export target already exists as an unchanged generated draft: {}; pass --overwrite-generated to replace it",
            target.display()
        );
    }
    Ok(Some(existing))
}

fn rollback_unregistered_draft(result: &ProcedureExportWriteResult, rendered: &str) -> Result<()> {
    let current = match std::fs::read(&result.path) {
        Ok(current) => current,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read draft before rollback {}", result.path.display()));
        }
    };
    if current != rendered.as_bytes() {
        bail!(
            "refusing to roll back procedure draft {} because it changed after export write",
            result.path.display()
        );
    }

    if let Some(previous_content) = &result.previous_content {
        write_atomically(&result.path, previous_content)
            .with_context(|| format!("restore previous procedure draft {}", result.path.display()))
    } else {
        std::fs::remove_file(&result.path).with_context(|| {
            format!(
                "remove unregistered procedure draft {}",
                result.path.display()
            )
        })
    }
}

fn same_generated_draft_except_generated_at(existing: &str, rendered: &str) -> bool {
    existing.contains(PROCEDURE_EXPORT_DRAFT_MARKER)
        && rendered.contains(PROCEDURE_EXPORT_DRAFT_MARKER)
        && normalize_generated_at(existing) == normalize_generated_at(rendered)
}

fn normalize_generated_at(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for line in value.split_inclusive('\n') {
        if is_generated_provenance_line(line) {
            output.push_str(generated_provenance_placeholder(line));
            if line.ends_with('\n') {
                output.push('\n');
            }
        } else {
            output.push_str(line);
        }
    }
    output
}

fn is_generated_provenance_line(line: &str) -> bool {
    let trimmed = line.strip_suffix('\n').unwrap_or(line);
    ((trimmed.starts_with(GENERATED_AT_PREFIX) && trimmed.len() > GENERATED_AT_PREFIX.len())
        || (trimmed.starts_with(REMEM_VERSION_PREFIX)
            && trimmed.len() > REMEM_VERSION_PREFIX.len()))
        && trimmed.ends_with('`')
}

fn generated_provenance_placeholder(line: &str) -> &'static str {
    if line
        .strip_suffix('\n')
        .unwrap_or(line)
        .starts_with(GENERATED_AT_PREFIX)
    {
        "- Generated at: `<generated-at>`"
    } else {
        "- remem version: `<remem-version>`"
    }
}

fn write_atomically(target: &Path, rendered: &str) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("procedure export target has no parent directory"))?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("procedure export target file name is not valid UTF-8"))?;
    let tmp = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::write(&tmp, rendered)
        .with_context(|| format!("write temporary procedure export {}", tmp.display()))?;
    std::fs::rename(&tmp, target)
        .with_context(|| format!("replace procedure export {}", target.display()))?;
    Ok(())
}

fn reject_high_context_path(path: &Path) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory for procedure export")?;
    reject_high_context_path_with_cwd(path, &cwd)
}

fn reject_high_context_path_with_cwd(path: &Path, cwd: &Path) -> Result<()> {
    let absolute = normalize_path_lexically(&absolute_path(path, cwd));
    reject_high_context_components(&absolute)?;
    reject_repo_skill_roots(&absolute, cwd)?;
    if let Ok(resolved) = resolve_existing_prefix(&absolute) {
        let resolved = normalize_path_lexically(&resolved);
        reject_high_context_components(&resolved)?;
        reject_repo_skill_roots(&resolved, cwd)?;
    }
    Ok(())
}

fn absolute_path(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    cwd.join(path)
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn resolve_existing_prefix(path: &Path) -> Result<PathBuf> {
    let mut prefix = path;
    while !prefix.exists() {
        prefix = prefix
            .parent()
            .ok_or_else(|| anyhow::anyhow!("procedure export path has no existing parent"))?;
    }
    let resolved_prefix = prefix
        .canonicalize()
        .with_context(|| format!("canonicalize procedure export path {}", prefix.display()))?;
    let suffix = path.strip_prefix(prefix).unwrap_or_else(|_| Path::new(""));
    Ok(resolved_prefix.join(suffix))
}

fn reject_high_context_components(path: &Path) -> Result<()> {
    let mut previous: Option<String> = None;
    for component in path.components() {
        let Component::Normal(raw) = component else {
            continue;
        };
        let value = raw.to_string_lossy();
        if value.eq_ignore_ascii_case(".claude") || value.eq_ignore_ascii_case(".codex") {
            bail!(
                "procedure export refuses high-context agent path {}; choose a neutral --out directory such as ./remem-drafts",
                path.display()
            );
        }
        if value.eq_ignore_ascii_case("AGENTS.md") || value.eq_ignore_ascii_case("CLAUDE.md") {
            bail!(
                "procedure export refuses high-context instruction file {}; choose --out <new-dir> instead",
                path.display()
            );
        }
        if previous
            .as_deref()
            .is_some_and(|previous| previous.eq_ignore_ascii_case(".agents"))
            && value.eq_ignore_ascii_case("skills")
        {
            bail!(
                "procedure export refuses skill-root path {}; review the draft in a neutral directory before moving it manually",
                path.display()
            );
        }
        previous = Some(value.into_owned());
    }
    Ok(())
}

fn reject_repo_skill_roots(path: &Path, cwd: &Path) -> Result<()> {
    for repo_root in protected_repo_roots(cwd, path) {
        let roots = [
            repo_root.join("skills"),
            repo_root.join(".agents").join("skills"),
        ];
        for root in roots {
            if path_starts_with_case_insensitive(path, &root) {
                bail!(
                    "procedure export refuses skill-root path {}; review the draft in a neutral directory before moving it manually",
                    path.display()
                );
            }
        }
        if path_is_under_plugin_skill_root(path, &repo_root) {
            bail!(
                "procedure export refuses skill-root path {}; review the draft in a neutral directory before moving it manually",
                path.display()
            );
        }
    }
    Ok(())
}

fn protected_repo_roots(cwd: &Path, target: &Path) -> Vec<PathBuf> {
    let mut repo_roots = Vec::new();
    for candidate in [cwd, target] {
        if let Some(repo_root) = discover_repo_root(candidate) {
            repo_roots.push(normalize_path_lexically(&repo_root));
        }
    }
    repo_roots.sort();
    repo_roots.dedup();
    repo_roots
}

fn path_is_under_plugin_skill_root(path: &Path, repo_root: &Path) -> bool {
    let Some(relative) = strip_prefix_case_insensitive(path, repo_root) else {
        return false;
    };
    let components: Vec<String> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    if components.len() < 3 {
        return false;
    }
    components[0] == "plugins" && components[2] == "skills"
}

fn strip_prefix_case_insensitive(path: &Path, prefix: &Path) -> Option<PathBuf> {
    if !path_starts_with_case_insensitive(path, prefix) {
        return None;
    }
    let prefix_len = prefix.components().count();
    let mut relative = PathBuf::new();
    for component in path.components().skip(prefix_len) {
        if let Component::Normal(value) = component {
            relative.push(value);
        }
    }
    Some(relative)
}

fn path_starts_with_case_insensitive(path: &Path, prefix: &Path) -> bool {
    let mut path_components = path.components();
    for prefix_component in prefix.components() {
        let Some(path_component) = path_components.next() else {
            return false;
        };
        if !components_equal_case_insensitive(path_component, prefix_component) {
            return false;
        }
    }
    true
}

fn components_equal_case_insensitive(left: Component<'_>, right: Component<'_>) -> bool {
    match (left, right) {
        (Component::Normal(left), Component::Normal(right)) => left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy()),
        _ => left == right,
    }
}

fn discover_repo_root(cwd: &Path) -> Option<PathBuf> {
    for candidate in cwd.ancestors() {
        if candidate.join(".git").exists() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests;
