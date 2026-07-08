use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{
    db,
    memory::procedure::{
        load_export_eligible_procedure, procedure_export_slug, render_procedure_export,
        ProcedureExportFormat, ProcedureExportSource,
    },
};

const DEFAULT_DRAFT_DIR: &str = "remem-drafts";

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
    if existing != rendered {
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
    let absolute = absolute_path(path)?;
    reject_high_context_components(&absolute)?;
    reject_repo_skill_roots(&absolute)?;
    if let Ok(resolved) = resolve_existing_prefix(&absolute) {
        reject_high_context_components(&resolved)?;
        reject_repo_skill_roots(&resolved)?;
    }
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(std::env::current_dir()
        .context("resolve current directory for procedure export")?
        .join(path))
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
        if value == ".claude" || value == ".codex" {
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
        if previous.as_deref() == Some(".agents") && value == "skills" {
            bail!(
                "procedure export refuses skill-root path {}; review the draft in a neutral directory before moving it manually",
                path.display()
            );
        }
        previous = Some(value.into_owned());
    }
    Ok(())
}

fn reject_repo_skill_roots(path: &Path) -> Result<()> {
    let cwd = std::env::current_dir().context("resolve current directory for procedure export")?;
    for root in [cwd.join("skills"), cwd.join(".agents").join("skills")] {
        if path.starts_with(&root) {
            bail!(
                "procedure export refuses skill-root path {}; review the draft in a neutral directory before moving it manually",
                path.display()
            );
        }
    }
    Ok(())
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
    fn writer_overwrites_only_unchanged_generated_target_with_explicit_flag() -> Result<()> {
        let root = procedure_export_temp_dir("procedure-export-overwrite")?;
        let out = root.join("drafts");
        let target = export_target_path(
            &out,
            &writer_fixture_source(),
            ProcedureExportFormat::RunbookMd,
        );
        std::fs::create_dir_all(target.parent().unwrap())?;
        std::fs::write(&target, RENDERED)?;

        let missing_flag = write_for(out.clone(), ProcedureExportFormat::RunbookMd, false)
            .expect_err("implicit overwrite must reject");
        assert!(missing_flag.to_string().contains("--overwrite-generated"));

        let result = write_for(out, ProcedureExportFormat::RunbookMd, true)?;

        assert!(result.overwritten);
        assert_eq!(std::fs::read_to_string(result.path)?, RENDERED);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }

    fn write_for(
        out_dir: PathBuf,
        format: ProcedureExportFormat,
        overwrite_generated: bool,
    ) -> Result<ProcedureExportWriteResult> {
        let source = writer_fixture_source();
        write_procedure_export_draft(ProcedureExportWriteRequest {
            source: &source,
            format,
            out_dir: Some(&out_dir),
            rendered: RENDERED,
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
