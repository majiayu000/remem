use std::process::Command;

fn install_status_temp_root() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "remem-install-status-{}-{}",
        std::process::id(),
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
