use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn install_status_temp_root() -> std::path::PathBuf {
    let counter = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "remem-install-status-{}-{}-{}",
        std::process::id(),
        counter,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    ))
}

#[test]
fn status_works_after_recommended_install_path() {
    let root = install_status_temp_root();
    let home = root.join("home");
    let data_dir = root.join("data");
    std::fs::create_dir_all(&home).expect("create temp home");

    let remem_bin = env!("CARGO_BIN_EXE_remem");
    let install = Command::new(remem_bin)
        .args(["install", "--target", "codex", "--hooks-only"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("REMEM_DATA_DIR", &data_dir)
        .env("REMEM_INSTALL_BINARY", remem_bin)
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("REMEM_CIPHER_KEY")
        .output()
        .expect("run remem install");

    assert!(
        install.status.success(),
        "install failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&install.stdout),
        String::from_utf8_lossy(&install.stderr)
    );
    assert!(data_dir.join(".key").exists(), "install should create key");
    assert!(
        data_dir.join("remem.db").exists(),
        "install should create database"
    );

    let status = Command::new(remem_bin)
        .args(["status", "--json"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("REMEM_DATA_DIR", &data_dir)
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("REMEM_CIPHER_KEY")
        .output()
        .expect("run remem status");

    assert!(
        status.status.success(),
        "status failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("status emits JSON");
    assert_eq!(
        report["database"]["path"].as_str(),
        Some(data_dir.join("remem.db").to_string_lossy().as_ref())
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn fresh_install_keeps_migration_info_out_of_normal_terminal_output() {
    let root = install_status_temp_root();
    let home = root.join("home");
    let data_dir = root.join("data");
    std::fs::create_dir_all(&home).expect("create temp home");

    let remem_bin = env!("CARGO_BIN_EXE_remem");
    let install = Command::new(remem_bin)
        .args(["install", "--target", "codex"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("REMEM_DATA_DIR", &data_dir)
        .env("REMEM_INSTALL_BINARY", remem_bin)
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("REMEM_CIPHER_KEY")
        .env_remove("REMEM_DEBUG")
        .env_remove("REMEM_STDERR_TO_LOG")
        .output()
        .expect("run remem install");

    let stdout = String::from_utf8_lossy(&install.stdout);
    let stderr = String::from_utf8_lossy(&install.stderr);
    assert!(
        install.status.success(),
        "install failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("[migrate]"),
        "normal install stderr should stay compact, got:\n{stderr}"
    );
    assert!(
        stderr.contains("  key    ->") && stderr.contains("  db     ->"),
        "install summary should remain visible, got:\n{stderr}"
    );

    let log = std::fs::read_to_string(data_dir.join("remem.log")).expect("read remem log");
    assert!(
        log.contains("[INFO] [migrate] applying"),
        "migration diagnostics should remain in remem.log, got:\n{log}"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn fresh_install_debug_keeps_migration_info_on_stderr() {
    let root = install_status_temp_root();
    let home = root.join("home");
    let data_dir = root.join("data");
    std::fs::create_dir_all(&home).expect("create temp home");

    let remem_bin = env!("CARGO_BIN_EXE_remem");
    let install = Command::new(remem_bin)
        .args(["install", "--target", "codex"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("REMEM_DATA_DIR", &data_dir)
        .env("REMEM_INSTALL_BINARY", remem_bin)
        .env("REMEM_DEBUG", "1")
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("REMEM_CIPHER_KEY")
        .env_remove("REMEM_STDERR_TO_LOG")
        .output()
        .expect("run remem install");

    let stdout = String::from_utf8_lossy(&install.stdout);
    let stderr = String::from_utf8_lossy(&install.stderr);
    assert!(
        install.status.success(),
        "install failed\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[INFO] [migrate] applying"),
        "debug install stderr should include migration diagnostics, got:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(root);
}
