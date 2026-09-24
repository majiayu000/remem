use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::db;
use crate::project_alias::{
    active_project_aliases, apply_project_alias_plan, preview_project_alias_plan, proof_sha256,
    revoke_project_alias, ProjectAliasApplyRequest, ProjectAliasPlanEntry, ProjectAliasProofKind,
};

use super::super::project_types::{ProjectAction, ProjectAliasAction};

pub(in crate::cli) fn run_project_action(action: ProjectAction) -> Result<()> {
    match action {
        ProjectAction::Alias { action } => match action {
            ProjectAliasAction::Add {
                alias_path,
                canonical,
                actor,
                reason,
                apply,
            } => add_alias(&alias_path, &canonical, &actor, &reason, apply),
            ProjectAliasAction::List => {
                let conn = db::open_db_read_only_current()?;
                print_json(&active_project_aliases(&conn, None)?)
            }
            ProjectAliasAction::Revoke {
                alias_path,
                actor,
                reason,
                apply,
            } => revoke_alias(&alias_path, &actor, &reason, apply),
        },
    }
}

fn add_alias(
    alias_path: &Path,
    canonical: &Path,
    actor: &str,
    reason: &str,
    apply: bool,
) -> Result<()> {
    let (entry, inventory_sha256) = build_alias_entry(alias_path, canonical)?;
    let entries = [entry.clone()];
    let request = ProjectAliasApplyRequest {
        source_inventory_sha256: &inventory_sha256,
        actor,
        reason,
        now_epoch: Utc::now().timestamp(),
        entries: &entries,
    };
    if apply {
        let conn = db::open_db_no_migrate()?;
        preview_project_alias_plan(&conn, &request)?;
        let current = build_alias_entry(alias_path, canonical)?;
        if current != (entry, inventory_sha256.clone()) {
            bail!("project alias Git proof changed before apply");
        }
        let result = apply_project_alias_plan(&conn, &request)?;
        print_json(
            &json!({"mode":"applied", "source_inventory_sha256":inventory_sha256, "result":result}),
        )
    } else {
        let conn = db::open_db_read_only_current()?;
        let result = preview_project_alias_plan(&conn, &request)?;
        print_json(
            &json!({"mode":"dry_run", "source_inventory_sha256":inventory_sha256, "proof":entry, "result":result}),
        )
    }
}

fn revoke_alias(alias_path: &Path, actor: &str, reason: &str, apply: bool) -> Result<()> {
    let alias = alias_path
        .to_str()
        .context("project alias path must be valid UTF-8")?;
    if !alias_path.is_absolute() {
        bail!("project alias revoke requires an absolute stored path");
    }
    let conn = if apply {
        db::open_db_no_migrate()?
    } else {
        db::open_db_read_only_current()?
    };
    let record = active_project_aliases(&conn, Some(alias))?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("active project alias not found: {alias}"))?;
    if apply {
        let revoked = revoke_project_alias(&conn, alias, actor, reason, Utc::now().timestamp())?;
        print_json(&json!({"mode":"applied", "revoked":revoked, "actor":actor, "reason":reason}))
    } else {
        if actor.trim().is_empty() || reason.trim().is_empty() {
            bail!("project alias revoke requires a non-empty actor and reason");
        }
        print_json(&json!({"mode":"dry_run", "revoke":record, "actor":actor, "reason":reason}))
    }
}

fn build_alias_entry(
    alias_path: &Path,
    canonical: &Path,
) -> Result<(ProjectAliasPlanEntry, String)> {
    let alias_root = project_root(alias_path)?;
    let canonical_root = project_root(canonical)?;
    if alias_root == canonical_root {
        bail!("project alias source and canonical path must differ");
    }
    let alias_head = git_stdout(&alias_root, &["rev-parse", "HEAD"])?;
    let canonical_head = git_stdout(&canonical_root, &["rev-parse", "HEAD"])?;
    let shared_commit = git_stdout(
        &canonical_root,
        &["merge-base", &alias_head, &canonical_head],
    )
    .context("project alias paths must share a Git commit")?;
    let proof_payload = json!({
        "from_path": alias_root,
        "to_path": canonical_root,
        "shared_commit_count": 1,
        "shared_commit": shared_commit,
    });
    let snapshot = json!({
        "alias_path": alias_root,
        "canonical_path": canonical_root,
        "alias_head": alias_head,
        "canonical_head": canonical_head,
        "shared_commit": shared_commit,
    });
    let source_inventory_sha256 = digest(&snapshot)?;
    Ok((
        ProjectAliasPlanEntry {
            alias_path: alias_root,
            canonical_path: canonical_root,
            proof_kind: ProjectAliasProofKind::GitCommitMembership,
            proof_sha256: proof_sha256(&proof_payload)?,
            proof_payload,
        },
        source_inventory_sha256,
    ))
}

fn project_root(path: &Path) -> Result<String> {
    let absolute = std::fs::canonicalize(path)
        .with_context(|| format!("project alias path does not exist: {}", path.display()))?;
    let root = db::project_from_cwd(
        absolute
            .to_str()
            .context("project alias path must be valid UTF-8")?,
    );
    Ok(root)
}

fn git_stdout(root: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()?;
    if !output.status.success() {
        bail!("cannot verify Git project identity at {root}");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn digest(value: &Value) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

fn print_json(value: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
