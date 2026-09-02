use std::{fs, path::Component, path::Path, process::Command};

use serde_json::Value;
use sha2::{Digest, Sha256};

const PRODUCTION_PATHSPEC_CONTRACT: &str = "eval/production-input-pathspec-v1.json";

fn main() {
    let (contract, paths) = production_pathspec();
    for path in &paths {
        println!("cargo:rerun-if-changed={path}");
    }
    emit_git_rerun_paths();

    if let Some(commit) = build_git_stdout(&["rev-parse", "--verify", "HEAD^{commit}"]) {
        println!("cargo:rustc-env=REMEM_BUILD_GIT_SHA={commit}");
    }
    if let Some(dirty) = git_dirty() {
        println!("cargo:rustc-env=REMEM_BUILD_SOURCE_DIRTY={dirty}");
    }
    println!("cargo:rustc-env=REMEM_BUILD_PRODUCTION_PATHSPEC_JSON={contract}");
    println!(
        "cargo:rustc-env=REMEM_BUILD_PRODUCTION_PATHSPEC_SHA256={:x}",
        Sha256::digest(contract.as_bytes())
    );
    if let Some(tree) = production_input_tree_sha256(&paths) {
        println!("cargo:rustc-env=REMEM_BUILD_PRODUCTION_INPUT_TREE_SHA256={tree}");
    }
}

fn production_pathspec() -> (String, Vec<String>) {
    let bytes = fs::read(PRODUCTION_PATHSPEC_CONTRACT)
        .unwrap_or_else(|error| panic!("read {PRODUCTION_PATHSPEC_CONTRACT}: {error}"));
    let value: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {PRODUCTION_PATHSPEC_CONTRACT}: {error}"));
    assert_eq!(
        value["schema_version"], 1,
        "unsupported production pathspec"
    );
    assert_eq!(
        value["contract_id"], "remem-production-input-pathspec-v1",
        "unexpected production pathspec contract"
    );
    let paths = value["paths"]
        .as_array()
        .expect("production pathspec paths must be an array")
        .iter()
        .map(|path| {
            path.as_str()
                .filter(|path| !path.is_empty())
                .expect("production pathspec entries must be non-empty strings")
                .to_string()
        })
        .collect::<Vec<_>>();
    assert!(!paths.is_empty(), "production pathspec must not be empty");
    assert!(
        paths.iter().all(|path| {
            let path = Path::new(path);
            !path.is_absolute()
                && !path
                    .components()
                    .any(|component| matches!(component, Component::ParentDir))
        }),
        "production pathspec entries must be repository-relative"
    );
    assert!(
        paths
            .iter()
            .any(|path| path == PRODUCTION_PATHSPEC_CONTRACT),
        "production pathspec must include its own contract"
    );
    let mut unique = paths.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(
        unique.len(),
        paths.len(),
        "production pathspec has duplicates"
    );
    (
        serde_json::to_string(&value).expect("serialize production pathspec"),
        paths,
    )
}

fn emit_git_rerun_paths() {
    for args in [
        &["rev-parse", "--git-path", "HEAD"][..],
        &["rev-parse", "--git-path", "index"][..],
    ] {
        if let Some(path) = build_git_stdout(args) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = build_git_stdout(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = build_git_stdout(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}

fn build_git_stdout(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn git_dirty() -> Option<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .ok()?;
    output.status.success().then_some(!output.stdout.is_empty())
}

fn production_input_tree_sha256(paths: &[String]) -> Option<String> {
    let mut args = vec!["ls-files", "-s", "--"];
    args.extend(paths.iter().map(String::as_str));
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(format!("{:x}", Sha256::digest(&output.stdout)))
}
