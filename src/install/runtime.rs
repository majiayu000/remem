use anyhow::{bail, Result};

use crate::install::host::{HookSupport, InstallTarget};
use crate::install::hosts::resolve_hosts;
use crate::install::paths::{binary_path, old_hooks_path, remem_data_dir};

pub fn install(target: InstallTarget, dry_run: bool) -> Result<()> {
    let bin = binary_path()?;
    let hosts = resolve_hosts(target);
    if hosts.is_empty() {
        bail!(
            "没检测到可用的 host（target=Auto 时仅安装已检测到的 host）。\n\
             如需强制安装到全部 host，请使用 `--target all`。"
        );
    }

    if dry_run {
        eprintln!("remem install (dry-run) — 以下写入不会被执行:");
        for host in &hosts {
            eprintln!("→ {}", host.name());
            for line in host.dry_run_plan(&bin) {
                eprintln!("{line}");
            }
        }
        eprintln!("  data   -> {}", remem_data_dir().display());
        return Ok(());
    }

    eprintln!("remem install:");
    for host in &hosts {
        eprintln!("→ {}", host.name());
        host.install_mcp(&bin)?;
        eprintln!("  MCP    -> {}", host.config_path().display());
        match host.install_hooks(&bin)? {
            HookSupport::Installed => eprintln!("  hooks  ✓"),
            HookSupport::Skipped(reason) => eprintln!("  hooks  skipped: {reason}"),
        }
    }

    let data_dir = remem_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    eprintln!("  data   -> {}", data_dir.display());
    eprintln!("  binary -> {}", bin);

    let old_path = old_hooks_path();
    if old_path.exists() {
        eprintln!();
        eprintln!("Legacy hooks.json detected: {}", old_path.display());
        eprintln!(
            "Claude Code does not read this file. Safe to delete: rm {}",
            old_path.display()
        );
    }

    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  1. Restart the affected host(s) (Claude Code / Codex)");
    eprintln!("  2. remem will automatically capture your sessions (hosts with hook support)");
    eprintln!("  3. Run 'remem status' to check system health");

    Ok(())
}

pub fn uninstall(target: InstallTarget, dry_run: bool) -> Result<()> {
    let bin = binary_path()?;
    // Uninstall defaults to "all known hosts" so a stale config isn't left
    // behind if the user removed a host before running uninstall.
    let effective = if matches!(target, InstallTarget::Auto) {
        InstallTarget::All
    } else {
        target
    };
    let hosts = resolve_hosts(effective);

    if dry_run {
        eprintln!("remem uninstall (dry-run) — 以下删除不会被执行:");
        for host in &hosts {
            eprintln!("→ {}: 移除 {}", host.name(), host.config_path().display());
        }
        return Ok(());
    }

    for host in &hosts {
        host.uninstall_mcp(&bin)?;
        host.uninstall_hooks(&bin)?;
        eprintln!("  {} 已清理 ({})", host.name(), host.config_path().display());
    }

    eprintln!("remem uninstall 完成");
    eprintln!("  数据目录 {} 保留不动", remem_data_dir().display());

    Ok(())
}
