use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::memory::scope_cleanup::{
    apply_memory_cleanup_plan, archive_objects, audit_scope, build_preference_cleanup_plan,
    memory_refs_from_ids, merge_preferences, parse_object_refs, reroute_objects, ArchiveRequest,
    MemoryCleanupApplyResult, MemoryCleanupPlan, MergePreferencesRequest, ObjectMutation,
    ObjectRef, RerouteRequest, ScopeAuditReport, ScopeAuditRequest, ScopeMutationResult,
    TargetProjectUpdate,
};
use crate::{db, memory};

use super::super::types::MemoryCleanupType;

pub(in crate::cli) fn run_audit_scope(project: Option<&str>, limit: i64, json: bool) -> Result<()> {
    let project = resolve_project(project);
    let conn = db::open_db()?;
    let report = audit_scope(
        &conn,
        &ScopeAuditRequest {
            project: &project,
            limit,
            now_epoch: chrono::Utc::now().timestamp(),
        },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    print_audit_report(&report);
    Ok(())
}

pub(in crate::cli) struct RerouteCliRequest<'a> {
    pub(in crate::cli) refs: &'a [String],
    pub(in crate::cli) ids: &'a [i64],
    pub(in crate::cli) owner_scope: &'a str,
    pub(in crate::cli) owner_key: &'a str,
    pub(in crate::cli) target_project: Option<&'a str>,
    pub(in crate::cli) clear_target_project: bool,
    pub(in crate::cli) topic_domain: Option<&'a str>,
    pub(in crate::cli) context_class: Option<&'a str>,
    pub(in crate::cli) confidence: Option<f64>,
    pub(in crate::cli) reason: Option<&'a str>,
    pub(in crate::cli) confirm: bool,
    pub(in crate::cli) dry_run: bool,
    pub(in crate::cli) json: bool,
}

pub(in crate::cli) fn run_reroute(req: RerouteCliRequest<'_>) -> Result<()> {
    let refs = collect_refs(req.refs, req.ids)?;
    let target_project = target_project_update(
        req.owner_scope,
        req.owner_key,
        req.target_project,
        req.clear_target_project,
    )?;
    let conn = db::open_db()?;
    let result = reroute_objects(
        &conn,
        &RerouteRequest {
            refs: &refs,
            owner_scope: req.owner_scope,
            owner_key: req.owner_key,
            target_project,
            topic_domain: req.topic_domain,
            context_class: req.context_class,
            routing_confidence: req.confidence,
            reason: req.reason,
            dry_run: req.dry_run,
            confirm: req.confirm,
        },
    )?;
    print_mutation_result("scope reroute", &result, req.json)
}

pub(in crate::cli) fn run_archive(
    refs: &[String],
    ids: &[i64],
    reason: Option<&str>,
    confirm: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    let refs = collect_refs(refs, ids)?;
    let conn = db::open_db()?;
    let result = archive_objects(
        &conn,
        &ArchiveRequest {
            refs: &refs,
            reason,
            dry_run,
            confirm,
        },
    )?;
    print_mutation_result("scope archive", &result, json)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::cli) fn run_memory_cleanup(
    cwd: Option<&str>,
    cleanup_type: Option<MemoryCleanupType>,
    all_types: bool,
    dry_run: bool,
    plan_out: Option<&Path>,
    apply: bool,
    plan: Option<&Path>,
    json: bool,
) -> Result<()> {
    if apply {
        if dry_run || plan_out.is_some() || cleanup_type.is_some() || all_types || cwd.is_some() {
            bail!("--apply only accepts --plan and optional --json");
        }
        let plan_path = plan.ok_or_else(|| anyhow::anyhow!("--apply requires --plan"))?;
        let raw = std::fs::read_to_string(plan_path)
            .with_context(|| format!("read cleanup plan {}", plan_path.display()))?;
        let cleanup_plan: MemoryCleanupPlan = serde_json::from_str(&raw)
            .with_context(|| format!("parse cleanup plan {}", plan_path.display()))?;
        let conn = db::open_db()?;
        let result = apply_memory_cleanup_plan(&conn, &cleanup_plan)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            print_cleanup_apply_result(&result);
        }
        return Ok(());
    }

    if plan.is_some() {
        bail!("--plan is only valid with --apply");
    }
    if !dry_run && plan_out.is_none() && !json {
        bail!("memory cleanup is dry-run-first; use --dry-run, --plan-out, or --json");
    }
    let cleanup_type = cleanup_type
        .or_else(|| all_types.then_some(MemoryCleanupType::Preference))
        .unwrap_or(MemoryCleanupType::Preference);
    if cleanup_type != MemoryCleanupType::Preference {
        bail!("unsupported cleanup type: {}", cleanup_type.as_str());
    }

    let cwd = crate::cli::cwd::resolve_cwd_arg(cwd.map(str::to_string));
    let project = db::project_from_cwd(&cwd);
    let conn = db::open_db()?;
    let cleanup_plan = build_preference_cleanup_plan(&conn, &project)?;
    if let Some(path) = plan_out {
        let json = serde_json::to_string_pretty(&cleanup_plan)?;
        std::fs::write(path, format!("{json}\n"))
            .with_context(|| format!("write cleanup plan {}", path.display()))?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&cleanup_plan)?);
    } else {
        print_cleanup_plan(&cleanup_plan, plan_out, all_types);
    }
    Ok(())
}

pub(in crate::cli) fn run_merge_preferences(
    project: Option<&str>,
    dry_run: bool,
    confirm: bool,
    json: bool,
) -> Result<()> {
    let project = resolve_project(project);
    let conn = db::open_db()?;
    let result = merge_preferences(
        &conn,
        &MergePreferencesRequest {
            project: &project,
            dry_run,
            confirm,
        },
    )?;
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    let mode = if result.dry_run { "dry-run" } else { "applied" };
    println!(
        "merge preferences {} project={} clusters={} affected={}",
        mode,
        project,
        result.clusters.len(),
        result.affected.len()
    );
    for cluster in result.clusters {
        println!(
            "  cluster={} canonical={} refs={}",
            cluster.cluster_key,
            cluster.canonical_ref,
            cluster.refs.join(",")
        );
        if let Some(content) = cluster.merged_content {
            println!("    merged={}", db::truncate_str(&content, 180));
        }
    }
    for mutation in result.affected {
        print_mutation(&mutation);
    }
    Ok(())
}

fn print_cleanup_plan(plan: &MemoryCleanupPlan, plan_out: Option<&Path>, all_types: bool) {
    let detector = if all_types {
        "preference (all supported detectors)"
    } else {
        "preference"
    };
    println!(
        "memory cleanup dry-run project={} detector={} groups={}",
        plan.project,
        detector,
        plan.groups.len()
    );
    for group in &plan.groups {
        println!(
            "  group={} owner={} type={} current=memory:{} stale=[{}] confidence={:.2}",
            group.cluster_key,
            owner_label(&group.owner_scope, &group.owner_key),
            group.memory_type,
            group.current_id,
            group
                .stale_ids
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(","),
            group.confidence
        );
        if let Some(state_key) = &group.state_key {
            println!("    state_key={state_key}");
        }
        println!("    reason={}", group.reason);
        if let Some(content) = &group.merged_content {
            println!("    preview={}", db::truncate_str(content, 180));
        }
    }
    if let Some(path) = plan_out {
        println!("plan written: {}", path.display());
        println!(
            "apply with: remem memory cleanup --apply --plan {}",
            path.display()
        );
    }
}

fn print_cleanup_apply_result(result: &MemoryCleanupApplyResult) {
    println!(
        "memory cleanup applied project={} groups={} current=[{}] stale=[{}] operations=[{}] edges={}",
        result.project,
        result.groups_applied,
        result
            .current_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
        result
            .stale_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
        result
            .operation_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
        result.edge_count
    );
    for mutation in &result.affected {
        print_mutation(mutation);
    }
}

fn resolve_project(project: Option<&str>) -> String {
    project.map(str::to_string).unwrap_or_else(|| {
        let cwd = crate::cli::cwd::resolve_cwd_arg(None);
        db::project_from_cwd(&cwd)
    })
}

fn collect_refs(ref_values: &[String], ids: &[i64]) -> Result<Vec<ObjectRef>> {
    let mut refs = parse_object_refs(ref_values)?;
    refs.extend(memory_refs_from_ids(ids)?);
    refs.sort_by_key(|object_ref| (object_ref.kind.as_str(), object_ref.id));
    refs.dedup();
    if refs.is_empty() {
        bail!("provide --refs object-qualified refs or --ids memory ids");
    }
    Ok(refs)
}

fn target_project_update(
    owner_scope: &str,
    owner_key: &str,
    target_project: Option<&str>,
    clear_target_project: bool,
) -> Result<TargetProjectUpdate> {
    let owner_scope = owner_scope.trim();
    let owner_key = owner_key.trim();
    if owner_key.is_empty() {
        bail!("owner-key must not be empty");
    }
    if clear_target_project && target_project.is_some() {
        bail!("use either --target-project or --clear-target-project, not both");
    }
    if let Some(target_project) = target_project {
        let target_project = target_project.trim();
        if target_project.is_empty() {
            bail!("target-project must not be empty; use --clear-target-project for SQL NULL");
        }
        return Ok(TargetProjectUpdate::Set(target_project.to_string()));
    }
    if clear_target_project || owner_scope != "repo" {
        return Ok(TargetProjectUpdate::Clear);
    }
    Ok(TargetProjectUpdate::Set(owner_key.to_string()))
}

fn print_mutation_result(prefix: &str, result: &ScopeMutationResult, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }
    let mode = if result.dry_run { "dry-run" } else { "applied" };
    println!(
        "{} {} action={} affected={}",
        prefix,
        mode,
        result.action,
        result.affected.len()
    );
    for mutation in &result.affected {
        print_mutation(mutation);
    }
    Ok(())
}

fn print_mutation(mutation: &ObjectMutation) {
    println!(
        "  {} {} -> {} owner={} -> {} title={}",
        mutation.object_ref,
        mutation.previous_status,
        mutation.new_status,
        owner_label(
            &mutation.previous_owner.owner_scope,
            &mutation.previous_owner.owner_key
        ),
        owner_label(
            &mutation.new_owner.owner_scope,
            &mutation.new_owner.owner_key
        ),
        mutation.title
    );
}

fn owner_label(scope: &Option<String>, key: &Option<String>) -> String {
    match (scope.as_deref(), key.as_deref()) {
        (Some(scope), Some(key)) => format!("{scope}:{key}"),
        (Some(scope), None) => format!("{scope}:<none>"),
        _ => "<legacy>".to_string(),
    }
}

fn print_audit_report(report: &ScopeAuditReport) {
    println!(
        "scope audit project={} limit={}",
        report.project, report.limit
    );
    print_bucket(
        "likely_correct_repo_memory",
        &report.likely_correct_repo_memory,
    );
    print_bucket(
        "likely_cross_tool_domain_pollution",
        &report.likely_cross_tool_domain_pollution,
    );
    print_duplicate_bucket("duplicate_preferences", &report.duplicate_preferences);
    print_duplicate_bucket("duplicate_workstreams", &report.duplicate_workstreams);
    print_bucket("stale_temporal_facts", &report.stale_temporal_facts);
    print_bucket("low_confidence_routing", &report.low_confidence_routing);
}

fn print_bucket(name: &str, items: &[memory::scope_cleanup::AuditItem]) {
    println!("{}: {}", name, items.len());
    for item in items {
        let owner = owner_label(&item.owner_scope, &item.owner_key);
        let suggestion = match (&item.suggested_owner_scope, &item.suggested_owner_key) {
            (Some(scope), Some(key)) => format!(" suggested={scope}:{key}"),
            _ => String::new(),
        };
        println!(
            "  {} action={} owner={}{} title={}",
            item.object_ref,
            item.suggested_action.as_deref().unwrap_or("review"),
            owner,
            suggestion,
            item.title
        );
    }
}

fn print_duplicate_bucket(name: &str, clusters: &[memory::scope_cleanup::DuplicateCluster]) {
    println!("{}: {}", name, clusters.len());
    for cluster in clusters {
        println!(
            "  cluster={} canonical={} refs={}",
            cluster.cluster_key,
            cluster.canonical_ref,
            cluster.refs.join(",")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_project_update_rejects_clear_and_set_conflict() {
        let err = target_project_update("repo", "/tmp/stash", Some("/tmp/stash"), true)
            .expect_err("clear and set should conflict");
        assert!(err
            .to_string()
            .contains("use either --target-project or --clear-target-project"));
    }

    #[test]
    fn target_project_update_rejects_empty_target_project() {
        let err = target_project_update("repo", "/tmp/stash", Some("  "), false)
            .expect_err("empty target_project should fail");
        assert!(err.to_string().contains("target-project must not be empty"));
    }

    #[test]
    fn target_project_update_defaults_repo_to_trimmed_owner_key() -> Result<()> {
        let update = target_project_update(" repo ", " /tmp/stash ", None, false)?;
        assert_eq!(update, TargetProjectUpdate::Set("/tmp/stash".to_string()));
        Ok(())
    }

    #[test]
    fn target_project_update_defaults_tool_to_clear() -> Result<()> {
        let update = target_project_update(" tool ", " codex-cli ", None, false)?;
        assert_eq!(update, TargetProjectUpdate::Clear);
        Ok(())
    }
}
