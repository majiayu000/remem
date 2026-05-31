use std::process::Command;

#[test]
fn eval_e2e_runs_in_sandbox_without_touching_parent_data_dir() {
    let sentinel = std::env::temp_dir().join(format!(
        "remem-e2e-parent-sentinel-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&sentinel).expect("create sentinel dir");

    let output = Command::new(env!("CARGO_BIN_EXE_remem"))
        .args(["eval-e2e", "--json", "--k", "3"])
        .env("REMEM_DATA_DIR", &sentinel)
        .env("REMEM_ALLOW_PLAINTEXT_DB", "1")
        .output()
        .expect("run remem eval-e2e");

    assert!(
        output.status.success(),
        "eval-e2e failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !sentinel.join("remem.db").exists(),
        "eval-e2e must not open the parent REMEM_DATA_DIR"
    );

    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("eval-e2e emits JSON");
    assert_eq!(
        report["metadata"]["config"]["boundary"],
        "REST API /api/v1/memories + /api/v1/search"
    );
    assert_eq!(report["metadata"]["data_dir_kept"], false);
    assert_ne!(
        report["metadata"]["data_dir"].as_str(),
        Some(sentinel.to_string_lossy().as_ref())
    );
    assert_eq!(report["api_metrics"]["total_queries"], 4);
    assert_eq!(report["api_metrics"]["hit_count"], 4);

    let _ = std::fs::remove_dir_all(&sentinel);
}
