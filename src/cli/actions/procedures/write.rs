use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{
    db,
    memory::procedure::{
        load_export_eligible_procedure, procedure_export_slug, render_procedure_export,
        ProcedureExportFormat, ProcedureExportSource, PROCEDURE_EXPORT_DRAFT_MARKER,
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
    let rendered = render_procedure_export(&source, format, chrono::Utc::now().timestamp())?;
    let result = write_procedure_export_draft(ProcedureExportWriteRequest {
        source: &source,
        format,
        out_dir,
        rendered: &rendered,
        overwrite_generated,
    })?;

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
    out_dir: Option<&'a Path>,
    rendered: &'a str,
    overwrite_generated: bool,
}

#[derive(Debug)]
struct ProcedureExportWriteResult {
    path: PathBuf,
    overwritten: bool,
}

fn write_procedure_export_draft(
    request: ProcedureExportWriteRequest<'_>,
) -> Result<ProcedureExportWriteResult> {
    let out_dir = request
        .out_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DRAFT_DIR));
    reject_high_context_path(&out_dir)?;

    let target = export_target_path(&out_dir, request.source, request.format);
    reject_high_context_path(&target)?;
    let overwrote_existing =
        ensure_writable_target(&target, request.rendered, request.overwrite_generated)?;

    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("procedure export target has no parent directory"))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create procedure export directory {}", parent.display()))?;
    write_atomically(&target, request.rendered)?;

    Ok(ProcedureExportWriteResult {
        path: target,
        overwritten: overwrote_existing,
    })
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
) -> Result<bool> {
    if !target.exists() {
        return Ok(false);
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
    Ok(true)
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
    for root in protected_skill_roots(cwd, path)? {
        if path_starts_with_case_insensitive(path, &root) {
            bail!(
                "procedure export refuses skill-root path {}; review the draft in a neutral directory before moving it manually",
                path.display()
            );
        }
    }
    Ok(())
}

fn protected_skill_roots(cwd: &Path, target: &Path) -> Result<Vec<PathBuf>> {
    let mut repo_roots = Vec::new();
    for candidate in [cwd, target] {
        if let Some(repo_root) = discover_repo_root(candidate) {
            repo_roots.push(normalize_path_lexically(&repo_root));
        }
    }
    repo_roots.sort();
    repo_roots.dedup();

    let mut roots = Vec::new();
    for repo_root in repo_roots {
        roots.push(repo_root.join("skills"));
        roots.push(repo_root.join(".agents").join("skills"));

        let plugins_dir = repo_root.join("plugins");
        if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
            for entry in entries {
                let entry = entry.with_context(|| {
                    format!(
                        "read procedure export plugin skill root under {}",
                        plugins_dir.display()
                    )
                })?;
                let path = entry.path().join("skills");
                if path.exists() {
                    roots.push(path);
                }
            }
        }
    }

    roots.sort();
    roots.dedup();
    Ok(roots
        .into_iter()
        .map(|root| normalize_path_lexically(&root))
        .collect())
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
mod tests {
    use super::*;

    const RENDERED: &str = "<!-- remem-draft: procedure export, review before commit -->\nDraft\n";

    #[test]
    fn writer_creates_neutral_runbook_target() -> Result<()> {
        let root = procedure_export_temp_dir("procedure-export-new")?;
        let result = write_for(root.join("drafts"), ProcedureExportFormat::RunbookMd, false)?;

        assert_eq!(
            result.path.file_name().and_then(|name| name.to_str()),
            Some("cargo-test.runbook.md")
        );
        assert_eq!(std::fs::read_to_string(result.path)?, RENDERED);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn writer_refuses_high_context_paths() -> Result<()> {
        let root = procedure_export_temp_dir("procedure-export-high-context")?;

        let claude_err = write_for(
            root.join(".claude").join("skills"),
            ProcedureExportFormat::ClaudeSkill,
            false,
        )
        .expect_err("claude skill roots must reject");
        assert!(claude_err.to_string().contains("high-context agent path"));

        let agents_skill_err = write_for(
            root.join(".AGENTS").join("SKILLS"),
            ProcedureExportFormat::ClaudeSkill,
            false,
        )
        .expect_err("agent skill roots must reject case-insensitively");
        assert!(agents_skill_err.to_string().contains("skill-root path"));

        let case_err = write_for(
            root.join(".CLAUDE").join("skills"),
            ProcedureExportFormat::ClaudeSkill,
            false,
        )
        .expect_err("case variants of claude roots must reject");
        assert!(case_err.to_string().contains("high-context agent path"));

        let agents_err = write_for(
            root.join("AGENTS.md"),
            ProcedureExportFormat::RunbookMd,
            false,
        )
        .expect_err("AGENTS.md must reject");
        assert!(agents_err
            .to_string()
            .contains("high-context instruction file"));

        let skill_root_err = write_for(
            std::env::current_dir()?
                .join("skills")
                .join("procedure-export"),
            ProcedureExportFormat::RunbookMd,
            false,
        )
        .expect_err("repo-local skills root must reject");
        assert!(skill_root_err.to_string().contains("skill-root path"));

        let plugin_skill_err = write_for(
            std::env::current_dir()?
                .join("plugins")
                .join("remem")
                .join("skills"),
            ProcedureExportFormat::RunbookMd,
            false,
        )
        .expect_err("plugin skills root must reject");
        assert!(plugin_skill_err.to_string().contains("skill-root path"));

        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn path_guard_resolves_repo_root_and_parent_components_before_skill_checks() -> Result<()> {
        let root = procedure_export_temp_dir("procedure-export-repo-root")?;
        std::fs::write(root.join(".git"), "gitdir: /tmp/not-used\n")?;
        std::fs::create_dir_all(root.join("src"))?;
        std::fs::create_dir_all(root.join("skills"))?;
        std::fs::create_dir_all(root.join(".agents").join("skills"))?;
        std::fs::create_dir_all(root.join("plugins").join("remem").join("skills"))?;
        let outside = procedure_export_temp_dir("procedure-export-outside-repo")?;

        let subdir_err =
            reject_high_context_path_with_cwd(Path::new("../skills"), &root.join("src"))
                .expect_err("repo skills must reject from subdirectories");
        assert!(subdir_err.to_string().contains("skill-root path"));

        let parent_err = reject_high_context_path_with_cwd(
            &root
                .join(".agents")
                .join("missing")
                .join("..")
                .join("skills"),
            &root,
        )
        .expect_err("parent components must normalize before guard checks");
        assert!(parent_err.to_string().contains("skill-root path"));

        let absolute_skill_err = reject_high_context_path_with_cwd(&root.join("skills"), &outside)
            .expect_err("absolute repo skills must reject even when cwd is outside the repo");
        assert!(absolute_skill_err.to_string().contains("skill-root path"));

        let case_skill_err = reject_high_context_path_with_cwd(&root.join("SKILLS"), &outside)
            .expect_err("case variants of repo skills must reject");
        assert!(case_skill_err.to_string().contains("skill-root path"));

        let absolute_plugin_err = reject_high_context_path_with_cwd(
            &root.join("plugins").join("remem").join("skills"),
            &outside,
        )
        .expect_err("absolute plugin skills must reject even when cwd is outside the repo");
        assert!(absolute_plugin_err.to_string().contains("skill-root path"));

        let case_plugin_err = reject_high_context_path_with_cwd(
            &root.join("Plugins").join("remem").join("SKILLS"),
            &outside,
        )
        .expect_err("case variants of plugin skills must reject");
        assert!(case_plugin_err.to_string().contains("skill-root path"));

        std::fs::remove_dir_all(outside)?;
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn writer_refuses_user_edited_existing_target_even_with_overwrite_flag() -> Result<()> {
        let root = procedure_export_temp_dir("procedure-export-edited")?;
        let out = root.join("drafts");
        let target = export_target_path(
            &out,
            &writer_fixture_source(),
            ProcedureExportFormat::RunbookMd,
        );
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::write(&target, "reviewed edits\n")?;

        let err = write_for(out, ProcedureExportFormat::RunbookMd, true)
            .expect_err("user edited target must reject");

        assert!(err.to_string().contains("may be reviewed or user-edited"));
        assert_eq!(std::fs::read_to_string(target)?, "reviewed edits\n");
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn writer_overwrites_unchanged_generated_target_after_version_change() -> Result<()> {
        let old = RENDERED.replace("Draft\n", "Draft\n- remem version: `0.5.187`\n");
        let new = RENDERED.replace("Draft\n", "Draft\n- remem version: `0.5.188`\n");

        assert!(same_generated_draft_except_generated_at(&old, &new));
        Ok(())
    }

    #[test]
    fn writer_overwrites_only_unchanged_generated_target_with_explicit_flag() -> Result<()> {
        let root = procedure_export_temp_dir("procedure-export-overwrite")?;
        let out = root.join("drafts");
        let source = writer_fixture_source();
        let target = export_target_path(&out, &source, ProcedureExportFormat::RunbookMd);
        let old_rendered =
            render_procedure_export(&source, ProcedureExportFormat::RunbookMd, 1_700_000_000)?;
        let new_rendered =
            render_procedure_export(&source, ProcedureExportFormat::RunbookMd, 1_700_000_600)?;
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::write(&target, old_rendered)?;

        let missing_flag = write_rendered_for(
            out.clone(),
            &source,
            ProcedureExportFormat::RunbookMd,
            &new_rendered,
            false,
        )
        .expect_err("implicit overwrite must reject");
        assert!(missing_flag.to_string().contains("--overwrite-generated"));

        let result = write_rendered_for(
            out,
            &source,
            ProcedureExportFormat::RunbookMd,
            &new_rendered,
            true,
        )?;

        assert!(result.overwritten);
        assert_eq!(std::fs::read_to_string(result.path)?, new_rendered);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    fn write_for(
        out_dir: PathBuf,
        format: ProcedureExportFormat,
        overwrite_generated: bool,
    ) -> Result<ProcedureExportWriteResult> {
        let source = writer_fixture_source();
        write_rendered_for(out_dir, &source, format, RENDERED, overwrite_generated)
    }

    fn write_rendered_for(
        out_dir: PathBuf,
        source: &ProcedureExportSource,
        format: ProcedureExportFormat,
        rendered: &str,
        overwrite_generated: bool,
    ) -> Result<ProcedureExportWriteResult> {
        write_procedure_export_draft(ProcedureExportWriteRequest {
            source,
            format,
            out_dir: Some(&out_dir),
            rendered,
            overwrite_generated,
        })
    }

    fn writer_fixture_source() -> ProcedureExportSource {
        ProcedureExportSource {
            id: 42,
            project: "/tmp/remem".to_string(),
            branch: Some("main".to_string()),
            topic_key: Some("procedure-cargo-test".to_string()),
            title: "Procedure: cargo-test".to_string(),
            stored_title: "Procedure: cargo-test".to_string(),
            canonical_content: String::new(),
            workflow_key: "cargo-test".to_string(),
            command: "cargo test".to_string(),
            reuse_condition: "same project".to_string(),
            files_touched: vec!["src/lib.rs".to_string()],
            evidence_event_ids: vec![100, 101],
            verified_runs: 2,
            last_verification_epoch: 1_700_000_000,
            confidence: 0.86,
            source_updated_at_epoch: 1_700_000_100,
        }
    }

    fn procedure_export_temp_dir(name: &str) -> Result<PathBuf> {
        let path = std::env::temp_dir().join(format!(
            "remem-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }
}
