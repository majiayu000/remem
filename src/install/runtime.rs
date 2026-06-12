use std::{
    io::{ErrorKind, Read},
    path::Path,
};

use anyhow::{bail, ensure, Context, Result};

use crate::install::duplicates::{format_warning_lines, inspect_install_paths};
use crate::install::host::{HookSupport, InstallTarget};
use crate::install::hosts::resolve_hosts;
use crate::install::paths::{binary_path, old_hooks_path, remem_data_dir};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::install) struct RuntimeStoreReady {
    pub(in crate::install) key_path: std::path::PathBuf,
    pub(in crate::install) db_path: std::path::PathBuf,
    pub(in crate::install) created_key: bool,
    pub(in crate::install) encrypted_existing_db: bool,
}

pub fn install(target: InstallTarget, dry_run: bool, hooks_only: bool) -> Result<()> {
    let bin = binary_path()?;
    let hosts = resolve_hosts(target);
    if hosts.is_empty() {
        bail!(
            "没检测到可用的 host（target=Auto 时仅安装已检测到的 host）。\n\
             如需强制安装到全部 host，请使用 `--target all`。"
        );
    }

    if dry_run {
        if hooks_only {
            eprintln!("remem install --hooks-only (dry-run) — 以下写入不会被执行:");
        } else {
            eprintln!("remem install (dry-run) — 以下写入不会被执行:");
        }
        for host in &hosts {
            eprintln!("→ {}", host.name());
            let plan = host.dry_run_plan(&bin);
            for line in plan
                .iter()
                .filter(|line| !hooks_only || !line.contains("MCP"))
            {
                eprintln!("{line}");
            }
        }
        eprintln!(
            "  config -> {} (memory_ai host/profile defaults)",
            crate::runtime_config::config_path().display()
        );
        eprintln!("  data   -> {}", remem_data_dir().display());
        eprintln!(
            "  key    -> {} (create if missing)",
            remem_data_dir().join(".key").display()
        );
        eprintln!(
            "  db     -> {} (initialize encrypted database if missing)",
            crate::db::db_path().display()
        );
        print_install_path_warnings(&bin);
        return Ok(());
    }

    if hooks_only {
        eprintln!("remem install --hooks-only:");
    } else {
        eprintln!("remem install:");
    }
    let runtime_store = ensure_runtime_store_ready()?;
    eprintln!(
        "  key    -> {} ({})",
        runtime_store.key_path.display(),
        if runtime_store.created_key {
            "created"
        } else {
            "existing"
        }
    );
    eprintln!(
        "  db     -> {} ({})",
        runtime_store.db_path.display(),
        if runtime_store.encrypted_existing_db {
            "encrypted existing database"
        } else {
            "ready"
        }
    );

    let runtime_hosts = hosts
        .iter()
        .map(|host| runtime_host_name(host.name()))
        .collect::<Vec<_>>();
    let config_path = crate::runtime_config::ensure_config_for_hosts(&runtime_hosts)?;
    eprintln!("  config -> {}", config_path.display());
    for host in &hosts {
        eprintln!("→ {}", host.name());
        if hooks_only {
            eprintln!("  MCP    skipped (hooks-only)");
        } else {
            host.install_mcp(&bin)?;
            eprintln!("  MCP    -> {}", host.config_path().display());
        }
        match host.install_hooks(&bin)? {
            HookSupport::Installed => eprintln!("  hooks  ✓"),
            HookSupport::Skipped(reason) => eprintln!("  hooks  skipped: {reason}"),
        }
    }

    let data_dir = remem_data_dir();
    std::fs::create_dir_all(&data_dir)?;
    eprintln!("  data   -> {}", data_dir.display());
    let api_token_path = crate::api::ensure_api_token()?;
    eprintln!("  API    -> token {}", api_token_path.display());
    eprintln!("  binary -> {}", bin);
    print_install_path_warnings(&bin);

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

pub(in crate::install) fn ensure_runtime_store_ready() -> Result<RuntimeStoreReady> {
    let data_dir = crate::db::data_dir();
    let key_path = data_dir.join(".key");
    let db_path = crate::db::db_path();

    if key_path.exists() {
        ensure_env_key_matches_persisted_key_if_set(&key_path)?;
        let conn = crate::db::open_db().with_context(|| {
            format!(
                "open remem database with existing SQLCipher key {}; run `remem status` after fixing the reported database/key error",
                key_path.display()
            )
        })?;
        drop(conn);
        return Ok(RuntimeStoreReady {
            key_path,
            db_path,
            created_key: false,
            encrypted_existing_db: false,
        });
    }

    if std::env::var_os("REMEM_CIPHER_KEY").is_some() {
        bail!(
            "REMEM_CIPHER_KEY is set but {} is missing; unset REMEM_CIPHER_KEY and run `remem install` again to create a persistent key file, or write the same key to {} before running `remem status`",
            key_path.display(),
            key_path.display()
        );
    }

    let db_existed = db_path.exists();
    if db_existed {
        ensure_existing_db_can_be_encrypted_without_key(&db_path, &key_path)?;
    }

    let key = crate::db::generate_cipher_key().with_context(|| {
        format!(
            "create SQLCipher key file {}; run `remem encrypt` to initialize the encrypted database manually",
            key_path.display()
        )
    })?;
    let cipher_key = crate::db::CipherKey::Raw(key);

    if db_existed {
        let encrypted_path = db_path.with_extension("db.enc");
        let backup_path = db_path.with_extension("db.bak");
        let encrypted_existed = encrypted_path.exists();
        let backup_existed = backup_path.exists();
        if let Err(error) = crate::db::encrypt_database(&cipher_key) {
            let mut error = error.context(format!(
                "encrypt existing remem database {}; run `remem encrypt` manually and rerun `remem install`",
                db_path.display()
            ));
            if let Err(rollback_error) = rollback_generated_key_after_encrypt_failure(
                &key_path,
                &cipher_key,
                &db_path,
                encrypted_existed,
                backup_existed,
            ) {
                error = error.context(format!(
                    "rollback generated key after failed database encryption: {rollback_error}"
                ));
            }
            return Err(error);
        }
    }

    let conn = crate::db::open_db().with_context(|| {
        format!(
            "initialize encrypted remem database {}; run `remem encrypt` manually and rerun `remem install`",
            db_path.display()
        )
    })?;
    drop(conn);

    Ok(RuntimeStoreReady {
        key_path,
        db_path,
        created_key: true,
        encrypted_existing_db: db_existed,
    })
}

fn ensure_env_key_matches_persisted_key_if_set(key_path: &Path) -> Result<()> {
    let Some(env_key) = std::env::var_os("REMEM_CIPHER_KEY") else {
        return Ok(());
    };
    let env_key = env_key.to_string_lossy();
    if env_key.trim().is_empty() {
        return Ok(());
    }
    let env_key = crate::db::parse_cipher_key(&env_key).context("parse REMEM_CIPHER_KEY")?;
    let persisted = std::fs::read_to_string(key_path)
        .with_context(|| format!("read existing SQLCipher key file {}", key_path.display()))?;
    let persisted_key = crate::db::parse_cipher_key(&persisted)
        .with_context(|| format!("parse SQLCipher key file {}", key_path.display()))?;
    ensure!(
        env_key.is_some() && env_key == persisted_key,
        "REMEM_CIPHER_KEY does not match existing SQLCipher key file {}; unset REMEM_CIPHER_KEY or update it to the same persisted key before running `remem install`",
        key_path.display()
    );
    Ok(())
}

fn rollback_generated_key_after_encrypt_failure(
    key_path: &Path,
    generated_key: &crate::db::CipherKey,
    db_path: &Path,
    encrypted_existed_before: bool,
    backup_existed_before: bool,
) -> Result<()> {
    let mut errors = Vec::new();
    let encrypted_path = db_path.with_extension("db.enc");
    let backup_path = db_path.with_extension("db.bak");

    if !db_path.exists() && !backup_existed_before && backup_path.exists() {
        if let Err(error) = std::fs::rename(&backup_path, db_path) {
            errors.push(format!(
                "restore {} from {}: {}",
                db_path.display(),
                backup_path.display(),
                error
            ));
        }
    }

    if !encrypted_existed_before && encrypted_path.exists() {
        if let Err(error) = std::fs::remove_file(&encrypted_path) {
            errors.push(format!("remove {}: {}", encrypted_path.display(), error));
        }
    }

    match std::fs::read_to_string(key_path) {
        Ok(contents) if contents == generated_key.stored_value() => {
            if let Err(error) = std::fs::remove_file(key_path) {
                errors.push(format!("remove {}: {}", key_path.display(), error));
            }
        }
        Ok(_) => errors.push(format!(
            "leave {} because its contents changed after generation",
            key_path.display()
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => errors.push(format!("read {}: {}", key_path.display(), error)),
    }

    if errors.is_empty() {
        Ok(())
    } else {
        bail!("{}", errors.join("; "))
    }
}

fn ensure_existing_db_can_be_encrypted_without_key(db_path: &Path, key_path: &Path) -> Result<()> {
    let mut file = std::fs::File::open(db_path)
        .with_context(|| format!("open existing remem database {}", db_path.display()))?;
    let mut header = [0_u8; 16];
    if let Err(error) = file.read_exact(&mut header) {
        if error.kind() == ErrorKind::UnexpectedEof {
            bail!(
                "existing remem database {} is too small to identify and {} is missing; restore the matching key file, or move {} aside and run `remem install` again",
                db_path.display(),
                key_path.display(),
                db_path.display()
            );
        }
        return Err(error)
            .with_context(|| format!("read existing remem database {}", db_path.display()));
    }
    ensure!(
        &header == b"SQLite format 3\0",
        "existing remem database {} does not look like plaintext SQLite and {} is missing; restore the matching key file, or move {} aside and run `remem install` again",
        db_path.display(),
        key_path.display(),
        db_path.display()
    );

    let conn = rusqlite::Connection::open(db_path)
        .with_context(|| format!("open existing plaintext database {}", db_path.display()))?;
    ensure!(
        crate::db::can_read_schema(&conn),
        "existing remem database {} is not readable plaintext SQLite and {} is missing; restore the matching key file, or move {} aside and run `remem install` again",
        db_path.display(),
        key_path.display(),
        db_path.display()
    );
    Ok(())
}

fn print_install_path_warnings(bin: &str) {
    let report = inspect_install_paths(Some(std::path::Path::new(bin)));
    let lines = format_warning_lines(&report);
    if lines.is_empty() {
        return;
    }

    eprintln!();
    eprintln!("Install path warning:");
    for line in lines {
        eprintln!("{line}");
    }
}

fn runtime_host_name(install_host: &str) -> &'static str {
    match install_host {
        "claude" => crate::runtime_config::CLAUDE_HOST,
        "codex" => crate::runtime_config::CODEX_HOST,
        _ => "unknown",
    }
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
        eprintln!(
            "  {} 已清理 ({})",
            host.name(),
            host.config_path().display()
        );
    }

    eprintln!("remem uninstall 完成");
    eprintln!("  数据目录 {} 保留不动", remem_data_dir().display());

    Ok(())
}
