use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

#[path = "support/cli_startup.rs"]
mod cli_startup;

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
        .env_remove("CODEX_HOME")
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
        .env_remove("CODEX_HOME")
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
        .env_remove("CODEX_HOME")
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

fn isolated_codex_command(root: &std::path::Path, codex_home: &std::ffi::OsStr) -> Command {
    let remem_bin = env!("CARGO_BIN_EXE_remem");
    let mut command = Command::new(remem_bin);
    command
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("CODEX_HOME", codex_home)
        .env("REMEM_DATA_DIR", root.join("data"))
        .env("REMEM_INSTALL_BINARY", remem_bin)
        .env_remove("REMEM_CONFIG")
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("REMEM_ALLOW_PLAINTEXT_DB")
        .env_remove("REMEM_CIPHER_KEY")
        .env_remove("REMEM_DEBUG")
        .env_remove("REMEM_STDERR_TO_LOG");
    command
}

#[test]
fn codex_selected_profile_is_shared_by_install_doctor_and_uninstall() {
    let root = install_status_temp_root();
    let default_root = root.join("home/.codex");
    let selected = root.join("profiles/selected");
    std::fs::create_dir_all(&default_root).unwrap();
    std::fs::create_dir_all(&selected).unwrap();
    let default_config = "# default profile must remain byte-identical\nmodel = 'default-model'\n";
    let default_hooks = r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"default-profile-hook"}]}]}}"#;
    std::fs::write(default_root.join("config.toml"), default_config).unwrap();
    std::fs::write(default_root.join("hooks.json"), default_hooks).unwrap();
    std::fs::write(
        selected.join("config.toml"),
        "model = 'selected-model'\n[mcp_servers.other]\ncommand = 'other-command'\n",
    )
    .unwrap();
    std::fs::write(selected.join("hooks.json"), r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"selected-profile-hook"}]}]}}"#).unwrap();

    let dry_run = isolated_codex_command(&root, selected.as_os_str())
        .args(["install", "--target", "codex", "--dry-run"])
        .output()
        .unwrap();
    assert!(dry_run.status.success(), "{dry_run:?}");
    let plan = String::from_utf8_lossy(&dry_run.stderr);
    assert!(
        plan.contains(selected.join("config.toml").to_string_lossy().as_ref()),
        "{plan}"
    );
    assert!(
        plan.contains(selected.join("hooks.json").to_string_lossy().as_ref()),
        "{plan}"
    );
    assert!(
        !root.join("data").exists(),
        "dry-run must not initialize the store"
    );

    let install = isolated_codex_command(&root, selected.as_os_str())
        .args(["install", "--target", "auto"])
        .output()
        .unwrap();
    assert!(install.status.success(), "{install:?}");
    let config = std::fs::read_to_string(selected.join("config.toml")).unwrap();
    let hooks = std::fs::read_to_string(selected.join("hooks.json")).unwrap();
    assert!(
        config.contains("mcp_servers.remem") && config.contains("selected-model"),
        "{config}"
    );
    assert!(
        hooks.contains("selected-profile-hook") && hooks.contains("UserPromptSubmit"),
        "{hooks}"
    );

    let held_default = root.join("home/default-codex-held");
    std::fs::rename(&default_root, &held_default).unwrap();
    assert!(!default_root.exists());
    let doctor = isolated_codex_command(&root, selected.as_os_str())
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).expect("doctor JSON");
    let checks = report["checks"].as_array().unwrap();
    for name in ["Hooks (codex)", "MCP (codex)"] {
        let check = checks
            .iter()
            .find(|check| check["name"] == name)
            .expect("Codex check");
        assert_eq!(check["status"], "ok", "{check}");
        assert!(
            check["detail"]
                .as_str()
                .unwrap()
                .contains(selected.to_string_lossy().as_ref()),
            "{check}"
        );
    }
    let capability = checks
        .iter()
        .find(|check| check["name"] == "Capture capability (codex)")
        .expect("selected Codex capture capability without a default profile");
    assert_eq!(capability["status"], "ok", "{capability}");
    assert!(capability["detail"]
        .as_str()
        .unwrap()
        .contains("drain-only"));
    std::fs::rename(&held_default, &default_root).unwrap();

    let uninstall = isolated_codex_command(&root, selected.as_os_str())
        .args(["uninstall", "--target", "codex"])
        .output()
        .unwrap();
    assert!(uninstall.status.success(), "{uninstall:?}");
    let config = std::fs::read_to_string(selected.join("config.toml")).unwrap();
    let hooks = std::fs::read_to_string(selected.join("hooks.json")).unwrap();
    assert!(
        !config.contains("mcp_servers.remem") && config.contains("other-command"),
        "{config}"
    );
    assert!(
        hooks.contains("selected-profile-hook") && !hooks.contains("remem context"),
        "{hooks}"
    );
    assert_eq!(
        std::fs::read_to_string(default_root.join("config.toml")).unwrap(),
        default_config
    );
    assert_eq!(
        std::fs::read_to_string(default_root.join("hooks.json")).unwrap(),
        default_hooks
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_codex_home_fails_before_install_or_uninstall_writes() {
    let root = install_status_temp_root();
    let default_root = root.join("home/.codex");
    std::fs::create_dir_all(&default_root).unwrap();
    let original = "[mcp_servers.remem]\ncommand = 'remem'\n";
    std::fs::write(default_root.join("config.toml"), original).unwrap();
    let file_root = root.join("file-not-directory");
    std::fs::write(&file_root, "sentinel").unwrap();
    for invalid in [
        std::ffi::OsStr::new(""),
        std::ffi::OsStr::new("relative-profile"),
        file_root.as_os_str(),
    ] {
        for command in ["install", "uninstall"] {
            let output = isolated_codex_command(&root, invalid)
                .args([command, "--target", "codex"])
                .output()
                .unwrap();
            assert!(!output.status.success(), "{command} accepted {invalid:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .to_lowercase()
                    .contains("codex"),
                "{output:?}"
            );
            assert!(
                !root.join("data").exists(),
                "invalid selection must not initialize the store"
            );
            assert_eq!(
                std::fs::read_to_string(default_root.join("config.toml")).unwrap(),
                original
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_codex_home_keeps_claude_diagnostics_visible() {
    let root = install_status_temp_root();
    let claude = root.join("home/.claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(claude.join("settings.json"), r#"{"hooks":{}}"#).unwrap();
    std::fs::write(root.join("home/.claude.json"), r#"{"mcpServers":{}}"#).unwrap();
    let doctor = isolated_codex_command(&root, std::ffi::OsStr::new("relative-profile"))
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let doctor_output = format!(
        "doctor status: {}\nstdout:\n{}\nstderr:\n{}",
        doctor.status,
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout)
        .unwrap_or_else(|error| panic!("doctor JSON: {error}\n{doctor_output}"));
    let checks = report["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("doctor checks array missing\n{doctor_output}"));
    for name in ["Hooks (claude)", "MCP (claude)"] {
        let check = checks
            .iter()
            .find(|check| check["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} check\n{doctor_output}"));
        assert_eq!(check["status"], "fail", "{check}\n{doctor_output}");
        let detail = check["detail"]
            .as_str()
            .unwrap_or_else(|| panic!("missing {name} detail\n{doctor_output}"));
        assert!(detail.contains("claude"), "{check}\n{doctor_output}");
    }
    for name in ["Hooks (codex)", "MCP (codex)", "Capture capability (codex)"] {
        let check = checks
            .iter()
            .find(|check| check["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} check\n{doctor_output}"));
        assert_eq!(check["status"], "fail", "{check}\n{doctor_output}");
        let detail = check["detail"]
            .as_str()
            .unwrap_or_else(|| panic!("missing {name} detail\n{doctor_output}"));
        assert!(
            detail.contains("invalid Codex home"),
            "{check}\n{doctor_output}"
        );
    }
    let capability = checks
        .iter()
        .find(|check| check["name"] == "Capture capability (claude)")
        .unwrap_or_else(|| panic!("missing Capture capability (claude) check\n{doctor_output}"));
    assert_eq!(capability["status"], "ok", "{capability}\n{doctor_output}");
    assert!(
        !root.join("data").exists(),
        "doctor must not initialize a store\n{doctor_output}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_degraded_embedding_does_not_initialize_store_or_logs() {
    let root = install_status_temp_root();
    let codex_home = root.join("home/.codex");
    std::fs::create_dir_all(&codex_home).unwrap();
    let doctor = isolated_codex_command(&root, codex_home.as_os_str())
        .env("REMEM_EMBEDDINGS_PROVIDER", "api")
        .env("REMEM_EMBEDDINGS_FALLBACK", "feature-hash")
        .env(
            "REMEM_EMBEDDINGS_API_KEY_ENV",
            "REMEM_DOCTOR_TEST_MISSING_API_KEY",
        )
        .env_remove("REMEM_EMBEDDINGS_API_KEY")
        .env_remove("REMEM_EMBEDDING_API_KEY")
        .env_remove("REMEM_DOCTOR_TEST_MISSING_API_KEY")
        .env_remove("REMEM_EMBEDDINGS_MODEL_DIR")
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let output = format!(
        "doctor status: {}\nstdout:\n{}\nstderr:\n{}",
        doctor.status,
        String::from_utf8_lossy(&doctor.stdout),
        String::from_utf8_lossy(&doctor.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout)
        .unwrap_or_else(|error| panic!("doctor JSON: {error}\n{output}"));
    let embedding = report["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|check| check["name"] == "Embedding provider")
        .unwrap_or_else(|| panic!("missing embedding check\n{output}"));
    assert_eq!(embedding["status"], "warn", "{output}");
    assert!(
        embedding["detail"]
            .as_str()
            .unwrap()
            .contains("using fallback feature-hash"),
        "{output}"
    );
    let stderr = String::from_utf8_lossy(&doctor.stderr);
    assert!(stderr.contains("[ERROR] [embedding]"), "{output}");
    assert!(stderr.contains("using fallback feature-hash"), "{output}");
    assert!(
        !root.join("data").exists(),
        "doctor must not initialize the store or logs\n{output}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn doctor_provider_diagnostics_keep_data_directory_absent() {
    let cases = [
        (
            "api",
            "configured embedding provider api unavailable",
            "using fallback feature-hash",
        ),
        #[cfg(windows)]
        (
            "auto",
            "Windows local embedding security policy rejected the model root",
            "using feature-hash",
        ),
    ];
    for (provider, expected_reason, expected_resolution) in cases {
        let root = install_status_temp_root();
        let data = root.join("data");
        std::fs::create_dir_all(root.join("home")).unwrap();
        let doctor = isolated_codex_command(&root, std::ffi::OsStr::new("relative-profile"))
            .args(["doctor", "--json"])
            .env("REMEM_EMBEDDINGS_PROVIDER", provider)
            .env("REMEM_EMBEDDINGS_FALLBACK", "feature-hash")
            .env(
                "REMEM_EMBEDDINGS_API_KEY_ENV",
                "REMEM_TEST_MISSING_EMBEDDING_KEY",
            )
            .env_remove("REMEM_TEST_MISSING_EMBEDDING_KEY")
            .env_remove("REMEM_EMBEDDINGS_API_KEY")
            .env_remove("REMEM_EMBEDDING_API_KEY")
            .env_remove("REMEM_EMBEDDINGS_MODEL_DIR")
            .output()
            .unwrap();
        assert!(!doctor.status.success(), "{doctor:?}");
        let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).unwrap();
        let checks = report["checks"].as_array().unwrap();
        let embedding = checks
            .iter()
            .find(|check| check["name"] == "Embedding provider")
            .expect("embedding provider diagnostic");
        assert_eq!(embedding["status"], "warn", "{embedding}");
        assert!(
            embedding["detail"]
                .as_str()
                .unwrap()
                .contains(expected_reason),
            "{embedding}"
        );
        for name in ["Hooks (codex)", "MCP (codex)", "Capture capability (codex)"] {
            let check = checks.iter().find(|check| check["name"] == name).unwrap();
            assert_eq!(check["status"], "fail", "{check}");
            assert!(
                check["detail"]
                    .as_str()
                    .unwrap()
                    .contains("invalid Codex home"),
                "{check}"
            );
        }
        let stderr = String::from_utf8_lossy(&doctor.stderr);
        assert!(stderr.contains("[ERROR] [embedding]"), "{stderr}");
        assert!(stderr.contains(expected_reason), "{stderr}");
        assert!(stderr.contains(expected_resolution), "{stderr}");
        assert!(
            !data.exists(),
            "doctor must leave the data directory absent during provider diagnostics\n{stderr}"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
