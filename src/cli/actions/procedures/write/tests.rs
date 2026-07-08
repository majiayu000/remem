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

    let subdir_err = reject_high_context_path_with_cwd(Path::new("../skills"), &root.join("src"))
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

#[test]
fn writer_requires_registry_match_before_overwriting_existing_target() -> Result<()> {
    let conn = rusqlite::Connection::open_in_memory()?;
    crate::migrate::run_migrations(&conn)?;
    let root = procedure_export_temp_dir("procedure-export-registry-overwrite")?;
    let out = root.join("drafts");
    let cwd = root.as_path();
    let source = writer_fixture_source();
    let target = export_target_path(&out, &source, ProcedureExportFormat::RunbookMd);
    let rendered =
        render_procedure_export(&source, ProcedureExportFormat::RunbookMd, 1_700_000_000)?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("test procedure export target has no parent"))?;
    std::fs::create_dir_all(parent)?;
    std::fs::write(&target, &rendered)?;

    let missing = verify_existing_target_registry(
        &conn,
        &source,
        ProcedureExportFormat::RunbookMd,
        &target,
        cwd,
        true,
    )
    .expect_err("missing registry row must block overwrite");
    assert!(missing
        .to_string()
        .contains("without a matching registry row"));

    record_procedure_export(
        &conn,
        ProcedureExportRecordRequest {
            source: &source,
            format: ProcedureExportFormat::RunbookMd,
            output_path: &target,
            content: &rendered,
            cwd,
            exported_at_epoch: 1_700_000_000,
        },
    )?;
    verify_existing_target_registry(
        &conn,
        &source,
        ProcedureExportFormat::RunbookMd,
        &target,
        cwd,
        true,
    )?;

    std::fs::write(&target, "reviewed edits\n")?;
    let mismatch = verify_existing_target_registry(
        &conn,
        &source,
        ProcedureExportFormat::RunbookMd,
        &target,
        cwd,
        true,
    )
    .expect_err("digest mismatch must block overwrite");
    assert!(mismatch.to_string().contains("digest no longer matches"));
    verify_existing_target_registry(
        &conn,
        &source,
        ProcedureExportFormat::RunbookMd,
        &target,
        cwd,
        false,
    )?;

    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn rollback_unregistered_draft_removes_new_file() -> Result<()> {
    let root = procedure_export_temp_dir("procedure-export-rollback-new")?;
    let result = write_for(root.join("drafts"), ProcedureExportFormat::RunbookMd, false)?;

    rollback_unregistered_draft(&result, RENDERED)?;

    assert!(!result.path.exists());
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn rollback_unregistered_draft_restores_overwritten_generated_file() -> Result<()> {
    let root = procedure_export_temp_dir("procedure-export-rollback-overwrite")?;
    let out = root.join("drafts");
    let source = writer_fixture_source();
    let target = export_target_path(&out, &source, ProcedureExportFormat::RunbookMd);
    let old_rendered =
        render_procedure_export(&source, ProcedureExportFormat::RunbookMd, 1_700_000_000)?;
    let new_rendered =
        render_procedure_export(&source, ProcedureExportFormat::RunbookMd, 1_700_000_600)?;
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("test procedure export target has no parent"))?;
    std::fs::create_dir_all(parent)?;
    std::fs::write(&target, &old_rendered)?;

    let result = write_rendered_for(
        out,
        &source,
        ProcedureExportFormat::RunbookMd,
        &new_rendered,
        true,
    )?;

    rollback_unregistered_draft(&result, &new_rendered)?;

    assert_eq!(std::fs::read_to_string(result.path)?, old_rendered);
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
        out_dir: &out_dir,
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
